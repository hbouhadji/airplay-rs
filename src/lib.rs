use std::{
    borrow::Cow,
    convert::Infallible,
    error::Error,
    fmt,
    net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
    sync::{
        Arc, Weak,
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
};

use rairplay::{
    ServiceFactory,
    config::{self, Config, DefaultKeychain, Features, Pairing},
    playback::{
        ChannelHandle, Device, Stream,
        audio::{AudioPacket, AudioParams},
        null::NullDevice,
        video::{PacketKind, VideoDevice, VideoPacket, VideoParams},
    },
    transport::DualStackListenerWithRtspRemap,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes},
};

mod discovery;

#[doc(hidden)]
pub enum AppEvent {
    VideoCommandQueued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoBackend {
    Auto,
    Native,
    GStreamer,
}

impl Default for VideoBackend {
    fn default() -> Self {
        Self::Auto
    }
}

impl VideoBackend {
    pub fn from_env_and_args<I, S>(args: I) -> Result<Self, Box<dyn Error>>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut backend = match std::env::var("AIRPLAY_RS_VIDEO_BACKEND") {
            Ok(value) => Self::parse_name(&value)?,
            Err(std::env::VarError::NotPresent) => Self::Auto,
            Err(error) => return Err(Box::new(error)),
        };

        let mut args = args.into_iter().map(Into::into);
        while let Some(arg) = args.next() {
            if let Some(value) = arg.strip_prefix("--video-backend=") {
                backend = Self::parse_name(value)?;
            } else if arg == "--video-backend" {
                let Some(value) = args.next() else {
                    return Err(invalid_input("--video-backend requires a value"));
                };
                backend = Self::parse_name(&value)?;
            } else if arg == "--native-video" {
                backend = Self::Native;
            } else if arg == "--gstreamer-video" {
                backend = Self::GStreamer;
            } else {
                return Err(invalid_input(format!(
                    "unknown argument '{arg}'. Use --video-backend auto|native|gstreamer"
                )));
            }
        }

        backend.ensure_available()?;
        Ok(backend)
    }

    fn parse_name(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "auto" => Ok(Self::Auto),
            "native" | "macos" | "avfoundation" => Ok(Self::Native),
            "gstreamer" | "gst" => Ok(Self::GStreamer),
            other => Err(invalid_input(format!(
                "unknown video backend '{other}'. Expected auto, native, or gstreamer"
            ))),
        }
    }

    fn resolved(self) -> Self {
        match self {
            Self::Auto => default_video_backend(),
            other => other,
        }
    }

    fn ensure_available(self) -> Result<(), Box<dyn Error>> {
        match self.resolved() {
            Self::Auto => unreachable!("video backend must be resolved before availability check"),
            Self::Native => {
                #[cfg(target_os = "macos")]
                {
                    Ok(())
                }

                #[cfg(not(target_os = "macos"))]
                {
                    Err(invalid_input(
                        "native video backend is only available on macOS",
                    ))
                }
            }
            Self::GStreamer => {
                #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
                {
                    Ok(())
                }

                #[cfg(all(target_os = "macos", not(feature = "macos-gstreamer")))]
                {
                    Err(invalid_input(
                        "GStreamer backend requested, but this binary was built without the 'macos-gstreamer' feature",
                    ))
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn default_video_backend() -> VideoBackend {
    VideoBackend::Native
}

#[cfg(not(target_os = "macos"))]
fn default_video_backend() -> VideoBackend {
    VideoBackend::GStreamer
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    ))
}

#[derive(Default)]
struct App {
    window: Option<Window>,
    video: Option<VideoSource>,
    airplay: Option<AirPlayServer>,
    proxy: Option<EventLoopProxy<AppEvent>>,
    video_backend: VideoBackend,
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("airplay-rs")
                    .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0)),
            )
            .expect("failed to create window");

        let video = VideoSource::start(
            &window,
            self.proxy
                .as_ref()
                .expect("event loop proxy must be initialized")
                .clone(),
            self.video_backend,
        );
        self.airplay = Some(AirPlayServer::start(video.device()));
        self.video = Some(video);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(video) = self.video.as_mut() {
                        video.drain_commands();
                    }
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(video) = self.video.as_mut() {
                    video.drain_commands();
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::VideoCommandQueued => {
                if let Some(video) = self.video.as_mut() {
                    video.drain_commands();
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(video) = self.video.as_mut() {
            video.drain_commands();
        }
    }

    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(video) = self.video.take() {
            video.stop();
        }
        self.airplay = None;
        self.window = None;
    }
}

struct VideoSource {
    device: AirPlayVideoDevice,
    receiver: Receiver<VideoCommand>,
    pipeline: Option<VideoPipeline>,
    window_handle: Option<u64>,
    video_backend: VideoBackend,
}

impl VideoSource {
    fn start(
        window: &Window,
        proxy: EventLoopProxy<AppEvent>,
        video_backend: VideoBackend,
    ) -> Self {
        let handle = native_window_handle(window);
        let (sender, receiver) = mpsc::channel();
        Self {
            device: AirPlayVideoDevice::new(sender, proxy),
            receiver,
            pipeline: None,
            window_handle: Some(handle as u64),
            video_backend,
        }
    }

    fn device(&self) -> AirPlayVideoDevice {
        self.device.clone()
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.receiver.try_recv() {
            if let Err(error) = self.handle_command(command) {
                eprintln!("video command failed: {error}");
            }
        }
    }

    fn handle_command(&mut self, command: VideoCommand) -> Result<(), VideoError> {
        match command {
            VideoCommand::Configure { codec, codec_data } => {
                if let Some(existing) = self.pipeline.take() {
                    existing.stop();
                }
                self.pipeline = Some(VideoPipeline::start(
                    self.video_backend,
                    codec,
                    codec_data,
                    self.window_handle,
                )?);
            }
            VideoCommand::Payload(payload) => {
                let Some(pipeline) = self.pipeline.as_mut() else {
                    eprintln!("dropping video payload before codec config");
                    return Ok(());
                };
                pipeline.push_payload(&payload)?;
            }
        }
        Ok(())
    }

    fn stop(mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.stop();
        }
    }
}

#[derive(Clone)]
struct AirPlayVideoDevice {
    sender: Sender<VideoCommand>,
    proxy: EventLoopProxy<AppEvent>,
}

impl AirPlayVideoDevice {
    fn new(sender: Sender<VideoCommand>, proxy: EventLoopProxy<AppEvent>) -> Self {
        Self { sender, proxy }
    }
}

impl Device for AirPlayVideoDevice {
    type Params = VideoParams;
    type Stream = AirPlayVideoStream;
    type Error = Infallible;

    async fn create(
        &self,
        id: u64,
        _params: Self::Params,
        _handle: Weak<dyn ChannelHandle>,
    ) -> Result<Self::Stream, Self::Error> {
        Ok(AirPlayVideoStream {
            id,
            sender: self.sender.clone(),
            proxy: self.proxy.clone(),
        })
    }
}

impl VideoDevice for AirPlayVideoDevice {}

struct AirPlayVideoStream {
    id: u64,
    sender: Sender<VideoCommand>,
    proxy: EventLoopProxy<AppEvent>,
}

impl Stream for AirPlayVideoStream {
    type Content = VideoPacket;

    fn on_data(&self, packet: Self::Content) {
        if let Err(error) = self.handle_packet(packet) {
            eprintln!("video packet failed for stream {}: {error}", self.id);
        }
    }

    fn on_ok(self) {
        eprintln!("AirPlay video stream {} finished", self.id);
    }

    fn on_err(self, err: Box<dyn Error>) {
        eprintln!("AirPlay video stream ended with an error: {err}");
    }
}

impl AirPlayVideoStream {
    fn handle_packet(&self, packet: VideoPacket) -> Result<(), VideoError> {
        match packet.kind {
            PacketKind::AvcC => self.send_command(VideoCommand::Configure {
                codec: Codec::H264,
                codec_data: packet.payload.to_vec(),
            }),
            PacketKind::HvcC => self.send_command(VideoCommand::Configure {
                codec: Codec::H265,
                codec_data: packet.payload.to_vec(),
            }),
            PacketKind::Payload => {
                self.send_command(VideoCommand::Payload(packet.payload.to_vec()))
            }
            PacketKind::Plist | PacketKind::Other(_) => Ok(()),
        }
    }

    fn send_command(&self, command: VideoCommand) -> Result<(), VideoError> {
        self.sender
            .send(command)
            .map_err(|error| VideoError::new(error.to_string()))?;
        let _ = self.proxy.send_event(AppEvent::VideoCommandQueued);
        Ok(())
    }
}

enum VideoCommand {
    Configure { codec: Codec, codec_data: Vec<u8> },
    Payload(Vec<u8>),
}

enum VideoPipeline {
    #[cfg(target_os = "macos")]
    Native(MacOsVideoPipeline),
    #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
    GStreamer(GStreamerPipeline),
}

impl VideoPipeline {
    fn start(
        backend: VideoBackend,
        codec: Codec,
        codec_data: impl AsRef<[u8]> + Send + 'static,
        window_handle: Option<u64>,
    ) -> Result<Self, VideoError> {
        match backend.resolved() {
            VideoBackend::Auto => unreachable!("video backend must be resolved before starting"),
            VideoBackend::Native => {
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::Native(MacOsVideoPipeline::start(
                        codec,
                        codec_data,
                        window_handle,
                    )?))
                }

                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (codec, codec_data, window_handle);
                    Err(VideoError::new(
                        "native video backend is only available on macOS",
                    ))
                }
            }
            VideoBackend::GStreamer => {
                #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
                {
                    Ok(Self::GStreamer(GStreamerPipeline::start(
                        codec,
                        codec_data,
                        window_handle,
                    )?))
                }

                #[cfg(all(target_os = "macos", not(feature = "macos-gstreamer")))]
                {
                    let _ = (codec, codec_data, window_handle);
                    Err(VideoError::new(
                        "GStreamer backend requested, but this binary was built without the 'macos-gstreamer' feature",
                    ))
                }
            }
        }
    }

    fn push_payload(&mut self, payload: &[u8]) -> Result<(), VideoError> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(pipeline) => pipeline.push_payload(payload),
            #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
            Self::GStreamer(pipeline) => pipeline.push_payload(payload),
        }
    }

    fn stop(self) {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(pipeline) => pipeline.stop(),
            #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
            Self::GStreamer(pipeline) => pipeline.stop(),
        }
    }
}

#[cfg(target_os = "macos")]
struct MacOsVideoPipeline {
    layer: objc2::rc::Retained<objc2::runtime::AnyObject>,
    format_description: CoreMediaRef<CMFormatDescription>,
    next_pts: i64,
}

#[cfg(target_os = "macos")]
impl MacOsVideoPipeline {
    fn start(
        codec: Codec,
        codec_data: impl AsRef<[u8]> + Send + 'static,
        window_handle: Option<u64>,
    ) -> Result<Self, VideoError> {
        use objc2::{class, msg_send, msg_send_id, runtime::AnyObject};

        let ns_view = window_handle.ok_or_else(|| VideoError::new("missing AppKit view handle"))?
            as *mut AnyObject;
        let format_description = codec.create_format_description(codec_data.as_ref())?;

        let layer = unsafe {
            let layer: objc2::rc::Retained<AnyObject> =
                msg_send_id![class!(AVSampleBufferDisplayLayer), new];
            let _: () = msg_send![ns_view, setWantsLayer: objc2::runtime::Bool::YES];
            let _: () = msg_send![ns_view, setLayer: &*layer];
            layer
        };

        Ok(Self {
            layer,
            format_description,
            next_pts: 0,
        })
    }

    fn push_payload(&mut self, payload: &[u8]) -> Result<(), VideoError> {
        use objc2::msg_send;

        let sample_buffer =
            create_sample_buffer(payload, self.format_description.as_ptr(), self.next_pts)?;
        self.next_pts += 1;

        unsafe {
            let _: () = msg_send![&*self.layer, enqueueSampleBuffer: sample_buffer.as_ptr()];
        }

        Ok(())
    }

    fn stop(self) {
        use objc2::msg_send;

        unsafe {
            let _: () = msg_send![&*self.layer, flushAndRemoveImage];
        }
    }
}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
struct GStreamerPipeline {
    appsrc: gstreamer_app::AppSrc,
    pipeline: gstreamer::Pipeline,
}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
impl GStreamerPipeline {
    fn start(
        codec: Codec,
        codec_data: impl AsRef<[u8]> + Send + 'static,
        window_handle: Option<u64>,
    ) -> Result<Self, VideoError> {
        use gstreamer as gst;
        use gstreamer::prelude::*;
        use gstreamer_video::prelude::VideoOverlayExtManual;

        init_gstreamer().expect("failed to initialize GStreamer");
        register_android_gstreamer_plugins();

        let pipeline = gst::parse::launch(&format!(
            "appsrc name=airplay_src is-live=true format=time do-timestamp=true block=true \
             ! {} disable-passthrough=true \
             ! vtdec_hw \
             ! video/x-raw(memory:GLMemory),format=NV12,texture-target=rectangle \
             ! glcolorconvert \
             ! glimagesink name=video_sink sync=false force-aspect-ratio=true",
            codec.parser_name(),
        ))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| VideoError::new("GStreamer launch did not create a pipeline"))?;

        let encoded_caps = codec.encoded_caps(codec_data.as_ref())?;
        let appsrc = pipeline
            .by_name("airplay_src")
            .ok_or_else(|| VideoError::new("missing AirPlay appsrc"))?
            .dynamic_cast::<gstreamer_app::AppSrc>()
            .map_err(|_| VideoError::new("airplay_src element has unexpected type"))?;
        appsrc.set_caps(Some(&encoded_caps));

        if let Some(window_handle) = window_handle {
            let overlay = pipeline
                .by_name("video_sink")
                .ok_or_else(|| VideoError::new("missing video sink"))?
                .dynamic_cast::<gstreamer_video::VideoOverlay>()
                .map_err(|_| VideoError::new("glimagesink does not implement GstVideoOverlay"))?;
            unsafe {
                overlay.set_window_handle(window_handle as _);
            }
        }

        pipeline.set_state(gst::State::Playing)?;

        Ok(Self { appsrc, pipeline })
    }

    fn push_payload(&self, payload: &[u8]) -> Result<(), VideoError> {
        let buffer = gst_buffer_from_slice(payload)?;
        self.appsrc.push_buffer(buffer)?;
        Ok(())
    }

    fn stop(self) {
        use gstreamer::prelude::*;

        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gstreamer::State::Null);
    }
}

#[derive(Debug, Clone, Copy)]
enum Codec {
    H264,
    H265,
}

impl Codec {
    #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
    fn parser_name(self) -> &'static str {
        match self {
            Self::H264 => "h264parse",
            Self::H265 => "h265parse",
        }
    }

    #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
    fn encoded_caps(self, codec_data: &[u8]) -> Result<gstreamer::Caps, VideoError> {
        let normalized = self.normalize_codec_data(codec_data);
        let codec_data = gst_buffer_from_slice(normalized.as_ref())?;

        Ok(match self {
            Self::H264 => gstreamer::Caps::builder("video/x-h264")
                .field("stream-format", "avc")
                .field("alignment", "au")
                .field("codec_data", codec_data)
                .build(),
            Self::H265 => gstreamer::Caps::builder("video/x-h265")
                .field("stream-format", "hvc1")
                .field("alignment", "au")
                .field("codec_data", codec_data)
                .build(),
        })
    }

    fn normalize_codec_data<'a>(self, codec_data: &'a [u8]) -> Cow<'a, [u8]> {
        match self {
            Self::H264 => Cow::Borrowed(codec_data),
            Self::H265 => find_mp4_box_payload(codec_data, b"hvcC")
                .map(Cow::Borrowed)
                .unwrap_or(Cow::Borrowed(codec_data)),
        }
    }

    #[cfg(target_os = "macos")]
    fn create_format_description(
        self,
        codec_data: &[u8],
    ) -> Result<CoreMediaRef<CMFormatDescription>, VideoError> {
        let normalized = self.normalize_codec_data(codec_data);
        let config = match self {
            Self::H264 => ParameterSets::from_avcc(normalized.as_ref())?,
            Self::H265 => ParameterSets::from_hvcc(normalized.as_ref())?,
        };
        let mut format_description = std::ptr::null_mut();
        let pointers = config.parameter_set_pointers();
        let sizes = config.parameter_set_sizes();

        let status = unsafe {
            match self {
                Self::H264 => CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    std::ptr::null(),
                    pointers.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    config.nal_unit_header_length as i32,
                    &mut format_description,
                ),
                Self::H265 => CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    std::ptr::null(),
                    pointers.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    config.nal_unit_header_length as i32,
                    std::ptr::null(),
                    &mut format_description,
                ),
            }
        };
        core_media_check(status, "create video format description")?;
        CoreMediaRef::new(format_description, "empty video format description")
    }
}

fn find_mp4_box_payload<'a>(payload: &'a [u8], box_type: &[u8; 4]) -> Option<&'a [u8]> {
    if payload.len() < 8 {
        return None;
    }

    for offset in 0..=payload.len().saturating_sub(8) {
        if &payload[offset + 4..offset + 8] != box_type {
            continue;
        }

        let size = u32::from_be_bytes(payload[offset..offset + 4].try_into().ok()?) as usize;
        if size < 8 {
            continue;
        }

        let box_end = offset.checked_add(size)?;
        if box_end > payload.len() {
            continue;
        }

        return Some(&payload[offset + 8..box_end]);
    }

    None
}

#[cfg(target_os = "macos")]
struct ParameterSets {
    nal_unit_header_length: u8,
    sets: Vec<Vec<u8>>,
}

#[cfg(target_os = "macos")]
impl ParameterSets {
    fn from_avcc(data: &[u8]) -> Result<Self, VideoError> {
        if data.len() < 7 {
            return Err(VideoError::new("invalid avcC codec data"));
        }

        let nal_unit_header_length = (data[4] & 0x03) + 1;
        let mut offset = 6;
        let mut sets = Vec::new();

        let sps_count = data[5] & 0x1f;
        for _ in 0..sps_count {
            sets.push(read_length_prefixed_parameter_set(data, &mut offset)?);
        }

        if offset >= data.len() {
            return Err(VideoError::new("avcC codec data is missing PPS entries"));
        }

        let pps_count = data[offset];
        offset += 1;
        for _ in 0..pps_count {
            sets.push(read_length_prefixed_parameter_set(data, &mut offset)?);
        }

        if sps_count == 0 || pps_count == 0 {
            return Err(VideoError::new("avcC codec data must include SPS and PPS"));
        }

        Ok(Self {
            nal_unit_header_length,
            sets,
        })
    }

    fn from_hvcc(data: &[u8]) -> Result<Self, VideoError> {
        if data.len() < 23 {
            return Err(VideoError::new("invalid hvcC codec data"));
        }

        let nal_unit_header_length = (data[21] & 0x03) + 1;
        let array_count = data[22];
        let mut offset = 23;
        let mut sets = Vec::new();
        let mut has_vps = false;
        let mut has_sps = false;
        let mut has_pps = false;

        for _ in 0..array_count {
            if offset + 3 > data.len() {
                return Err(VideoError::new("truncated hvcC parameter array"));
            }

            let nal_unit_type = data[offset] & 0x3f;
            offset += 1;
            let nal_count = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;

            for _ in 0..nal_count {
                let parameter_set = read_length_prefixed_parameter_set(data, &mut offset)?;
                match nal_unit_type {
                    32 => has_vps = true,
                    33 => has_sps = true,
                    34 => has_pps = true,
                    _ => {}
                }
                sets.push(parameter_set);
            }
        }

        if !has_vps || !has_sps || !has_pps {
            return Err(VideoError::new(
                "hvcC codec data must include VPS, SPS and PPS",
            ));
        }

        Ok(Self {
            nal_unit_header_length,
            sets,
        })
    }

    fn parameter_set_pointers(&self) -> Vec<*const u8> {
        self.sets
            .iter()
            .map(|parameter_set| parameter_set.as_ptr())
            .collect()
    }

    fn parameter_set_sizes(&self) -> Vec<usize> {
        self.sets
            .iter()
            .map(|parameter_set| parameter_set.len())
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn read_length_prefixed_parameter_set(
    data: &[u8],
    offset: &mut usize,
) -> Result<Vec<u8>, VideoError> {
    if *offset + 2 > data.len() {
        return Err(VideoError::new("truncated codec parameter set length"));
    }

    let length = u16::from_be_bytes([data[*offset], data[*offset + 1]]) as usize;
    *offset += 2;

    let end = offset
        .checked_add(length)
        .ok_or_else(|| VideoError::new("codec parameter set length overflow"))?;
    if end > data.len() {
        return Err(VideoError::new("truncated codec parameter set"));
    }

    let parameter_set = data[*offset..end].to_vec();
    *offset = end;
    Ok(parameter_set)
}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
fn gst_buffer_from_slice(data: &[u8]) -> Result<gstreamer::Buffer, VideoError> {
    let mut buffer = gstreamer::Buffer::with_size(data.len())?;
    let Some(buf) = buffer.get_mut() else {
        return Err(VideoError::new("failed to get writable GStreamer buffer"));
    };
    buf.copy_from_slice(0, data)
        .map_err(|_| VideoError::new("failed to copy data into GStreamer buffer"))?;
    Ok(buffer)
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CMSampleTimingInfo {
    duration: CMTime,
    presentation_time_stamp: CMTime,
    decode_time_stamp: CMTime,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CMBlockBuffer {
    inner: [u8; 0],
    _p: std::cell::UnsafeCell<
        std::marker::PhantomData<(*const std::cell::UnsafeCell<()>, std::marker::PhantomPinned)>,
    >,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CMFormatDescription {
    inner: [u8; 0],
    _p: std::cell::UnsafeCell<
        std::marker::PhantomData<(*const std::cell::UnsafeCell<()>, std::marker::PhantomPinned)>,
    >,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CMSampleBuffer {
    inner: [u8; 0],
    _p: std::cell::UnsafeCell<
        std::marker::PhantomData<(*const std::cell::UnsafeCell<()>, std::marker::PhantomPinned)>,
    >,
}

#[cfg(target_os = "macos")]
unsafe impl objc2::RefEncode for CMBlockBuffer {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("OpaqueCMBlockBuffer", &[]),
    );
}

#[cfg(target_os = "macos")]
unsafe impl objc2::RefEncode for CMFormatDescription {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("opaqueCMFormatDescription", &[]),
    );
}

#[cfg(target_os = "macos")]
unsafe impl objc2::RefEncode for CMSampleBuffer {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("opaqueCMSampleBuffer", &[]),
    );
}

#[cfg(target_os = "macos")]
struct CoreMediaRef<T> {
    ptr: *mut T,
}

#[cfg(target_os = "macos")]
impl<T> CoreMediaRef<T> {
    fn new(ptr: *mut T, empty_message: &'static str) -> Result<Self, VideoError> {
        if ptr.is_null() {
            Err(VideoError::new(empty_message))
        } else {
            Ok(Self { ptr })
        }
    }

    fn as_ptr(&self) -> *mut T {
        self.ptr
    }
}

#[cfg(target_os = "macos")]
impl<T> Drop for CoreMediaRef<T> {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.ptr.cast());
        }
    }
}

#[cfg(target_os = "macos")]
fn create_sample_buffer(
    payload: &[u8],
    format_description: *mut CMFormatDescription,
    pts: i64,
) -> Result<CoreMediaRef<CMSampleBuffer>, VideoError> {
    if payload.is_empty() {
        return Err(VideoError::new("empty video payload"));
    }

    let mut block_buffer = std::ptr::null_mut();
    let status = unsafe {
        CMBlockBufferCreateWithMemoryBlock(
            std::ptr::null(),
            std::ptr::null_mut(),
            payload.len(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            payload.len(),
            K_CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW_FLAG,
            &mut block_buffer,
        )
    };
    core_media_check(status, "create CMBlockBuffer")?;
    let block_buffer = CoreMediaRef::<CMBlockBuffer>::new(block_buffer, "empty CMBlockBuffer")?;

    let status = unsafe {
        CMBlockBufferReplaceDataBytes(
            payload.as_ptr().cast(),
            block_buffer.as_ptr().cast(),
            0,
            payload.len(),
        )
    };
    core_media_check(status, "copy video payload into CMBlockBuffer")?;

    let timing = CMSampleTimingInfo {
        duration: cm_time(1, 60),
        presentation_time_stamp: cm_time(pts, 60),
        decode_time_stamp: cm_time_invalid(),
    };
    let sample_size = payload.len();
    let mut sample_buffer = std::ptr::null_mut();
    let status = unsafe {
        CMSampleBufferCreateReady(
            std::ptr::null(),
            block_buffer.as_ptr().cast(),
            format_description.cast(),
            1,
            1,
            &timing,
            1,
            &sample_size,
            &mut sample_buffer,
        )
    };
    core_media_check(status, "create CMSampleBuffer")?;
    mark_sample_buffer_display_immediately(sample_buffer);
    CoreMediaRef::new(sample_buffer, "empty CMSampleBuffer")
}

#[cfg(target_os = "macos")]
fn mark_sample_buffer_display_immediately(sample_buffer: *mut CMSampleBuffer) {
    unsafe {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample_buffer, true);
        if attachments.is_null() {
            return;
        }

        let first_attachment = CFArrayGetValueAtIndex(attachments, 0);
        if first_attachment.is_null() {
            return;
        }

        CFDictionarySetValue(
            first_attachment,
            kCMSampleAttachmentKey_DisplayImmediately,
            kCFBooleanTrue,
        );
    }
}

#[cfg(target_os = "macos")]
fn cm_time(value: i64, timescale: i32) -> CMTime {
    CMTime {
        value,
        timescale,
        flags: K_CM_TIME_FLAGS_VALID,
        epoch: 0,
    }
}

#[cfg(target_os = "macos")]
fn cm_time_invalid() -> CMTime {
    CMTime {
        value: 0,
        timescale: 0,
        flags: 0,
        epoch: 0,
    }
}

#[cfg(target_os = "macos")]
fn core_media_check(status: i32, action: &'static str) -> Result<(), VideoError> {
    if status == 0 {
        Ok(())
    } else {
        Err(VideoError::new(format!(
            "{action} failed with OSStatus {status}"
        )))
    }
}

#[cfg(target_os = "macos")]
const K_CM_TIME_FLAGS_VALID: u32 = 1 << 0;

#[cfg(target_os = "macos")]
const K_CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW_FLAG: u32 = 1 << 0;

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const std::ffi::c_void;

    fn CFArrayGetValueAtIndex(
        the_array: *const std::ffi::c_void,
        idx: isize,
    ) -> *const std::ffi::c_void;
    fn CFDictionarySetValue(
        the_dict: *const std::ffi::c_void,
        key: *const std::ffi::c_void,
        value: *const std::ffi::c_void,
    );
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[cfg(target_os = "macos")]
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMSampleAttachmentKey_DisplayImmediately: *const std::ffi::c_void;

    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: *const std::ffi::c_void,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        format_description_out: *mut *mut CMFormatDescription,
    ) -> i32;

    fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        allocator: *const std::ffi::c_void,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        extensions: *const std::ffi::c_void,
        format_description_out: *mut *mut CMFormatDescription,
    ) -> i32;

    fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: *const std::ffi::c_void,
        memory_block: *mut std::ffi::c_void,
        block_length: usize,
        block_allocator: *const std::ffi::c_void,
        custom_block_source: *const std::ffi::c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        block_buffer_out: *mut *mut CMBlockBuffer,
    ) -> i32;

    fn CMBlockBufferReplaceDataBytes(
        source_bytes: *const std::ffi::c_void,
        destination_buffer: *mut std::ffi::c_void,
        offset_into_destination: usize,
        data_length: usize,
    ) -> i32;

    fn CMSampleBufferCreateReady(
        allocator: *const std::ffi::c_void,
        data_buffer: *mut std::ffi::c_void,
        format_description: *mut std::ffi::c_void,
        num_samples: isize,
        num_sample_timing_entries: isize,
        sample_timing_array: *const CMSampleTimingInfo,
        num_sample_size_entries: isize,
        sample_size_array: *const usize,
        sample_buffer_out: *mut *mut CMSampleBuffer,
    ) -> i32;

    fn CMSampleBufferGetSampleAttachmentsArray(
        sbuf: *mut CMSampleBuffer,
        create_if_necessary: bool,
    ) -> *const std::ffi::c_void;
}

#[derive(Debug)]
struct VideoError {
    message: String,
}

impl VideoError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for VideoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for VideoError {}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
impl From<gstreamer::glib::BoolError> for VideoError {
    fn from(error: gstreamer::glib::BoolError) -> Self {
        Self::new(error.to_string())
    }
}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
impl From<gstreamer::glib::Error> for VideoError {
    fn from(error: gstreamer::glib::Error) -> Self {
        Self::new(error.to_string())
    }
}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
impl From<gstreamer::StateChangeError> for VideoError {
    fn from(error: gstreamer::StateChangeError) -> Self {
        Self::new(error.to_string())
    }
}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
impl From<gstreamer::FlowError> for VideoError {
    fn from(error: gstreamer::FlowError) -> Self {
        Self::new(error.to_string())
    }
}

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
fn init_gstreamer() -> Result<(), VideoError> {
    gstreamer::init()?;

    for element in [
        "appsrc",
        "h264parse",
        "h265parse",
        "vtdec_hw",
        "capsfilter",
        "glcolorconvert",
        "glimagesink",
    ] {
        if gstreamer::ElementFactory::find(element).is_none() {
            return Err(VideoError::new(format!(
                "required GStreamer element '{element}' is missing"
            )));
        }
    }

    Ok(())
}

struct AirPlayServer {
    _mdns: mdns_sd::ServiceDaemon,
    _thread: JoinHandle<()>,
}

impl AirPlayServer {
    fn start(video_device: AirPlayVideoDevice) -> Self {
        let mut config = Config {
            mac_addr: Default::default(),
            features: Default::default(),
            manufacturer: env!("CARGO_PKG_AUTHORS").to_string(),
            model: env!("CARGO_PKG_NAME").to_string(),
            name: env!("CARGO_PKG_NAME").to_string(),
            fw_version: env!("CARGO_PKG_VERSION").to_string(),
            pin: None,
            pairing: Pairing::HomeKit,
            keychain: DefaultKeychain::default(),
            audio: config::Audio {
                device: NullDevice::<AudioParams, AudioPacket>::default(),
                ..Default::default()
            },
            video: config::Video {
                width: 1920,
                height: 1080,
                fps: 30,
                buf_size: 1024 * 1024,
                device: video_device,
            },
        };

        config.video.width = 3840;
        config.video.height = 2160;
        config.video.fps = 60;
        config.features |= Features::ScreenMultiCodec;

        match config.pairing {
            Pairing::Legacy => {
                config.features.insert(Features::LegacyPairing);
                config.features.remove(Features::HomeKitPairing);
            }
            Pairing::HomeKit => {
                config.features.insert(Features::HomeKitPairing);
                config.features.remove(Features::LegacyPairing);
            }
        }

        let config = Arc::new(config);
        let mdns = discovery::mdns_broadcast(&config, 7000);
        let thread_config = Arc::clone(&config);
        let thread = std::thread::spawn(move || {
            if let Err(err) = run_airplay_server(thread_config, 7000) {
                eprintln!("AirPlay server stopped: {err}");
            }
        });

        Self {
            _mdns: mdns,
            _thread: thread,
        }
    }
}

fn run_airplay_server(
    config: Arc<Config<NullDevice<AudioParams, AudioPacket>, AirPlayVideoDevice, DefaultKeychain>>,
    port: u16,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;

    rt.block_on(async move {
        let listener = DualStackListenerWithRtspRemap::bind(
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port),
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0),
        )?;

        axum::serve(listener, ServiceFactory::new(config)).await?;
        Ok(())
    })
}

#[cfg(target_os = "android")]
fn register_android_gstreamer_plugins() {
    unsafe extern "C" {
        fn gst_plugin_opengl_register();
    }

    unsafe {
        gst_plugin_opengl_register();
    }
}

#[cfg(all(
    not(target_os = "android"),
    any(not(target_os = "macos"), feature = "macos-gstreamer")
))]
fn register_android_gstreamer_plugins() {}

fn native_window_handle(window: &Window) -> usize {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match window
        .window_handle()
        .expect("failed to get native window handle")
        .as_raw()
    {
        RawWindowHandle::AppKit(handle) => handle.ns_view.as_ptr() as usize,
        RawWindowHandle::Win32(handle) => handle.hwnd.get() as usize,
        RawWindowHandle::Xlib(handle) => handle.window as usize,
        RawWindowHandle::Xcb(handle) => handle.window.get() as usize,
        RawWindowHandle::Wayland(handle) => handle.surface.as_ptr() as usize,
        RawWindowHandle::AndroidNdk(handle) => handle.a_native_window.as_ptr() as usize,
        other => panic!("unsupported native window handle for GstVideoOverlay: {other:?}"),
    }
}

pub fn run(event_loop: EventLoop<AppEvent>) -> Result<(), winit::error::EventLoopError> {
    run_with_video_backend(event_loop, VideoBackend::Auto)
}

pub fn run_with_video_backend(
    event_loop: EventLoop<AppEvent>,
    video_backend: VideoBackend,
) -> Result<(), winit::error::EventLoopError> {
    let mut app = App {
        proxy: Some(event_loop.create_proxy()),
        video_backend,
        ..Default::default()
    };
    event_loop.run_app(&mut app)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .with_android_app(app)
        .build()
        .expect("failed to create Android event loop");

    run(event_loop).expect("Android event loop failed");
}
