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

#[derive(Default)]
struct App {
    window: Option<Window>,
    video: Option<VideoSource>,
    airplay: Option<AirPlayServer>,
    proxy: Option<EventLoopProxy<AppEvent>>,
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
    pipeline: Option<GStreamerPipeline>,
    window_handle: Option<u64>,
}

impl VideoSource {
    fn start(window: &Window, proxy: EventLoopProxy<AppEvent>) -> Self {
        init_gstreamer().expect("failed to initialize GStreamer");
        register_android_gstreamer_plugins();

        let handle = native_window_handle(window);
        let (sender, receiver) = mpsc::channel();
        Self {
            device: AirPlayVideoDevice::new(sender, proxy),
            receiver,
            pipeline: None,
            window_handle: Some(handle as u64),
        }
    }

    fn device(&self) -> AirPlayVideoDevice {
        self.device.clone()
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.receiver.try_recv() {
            if let Err(error) = self.handle_command(command) {
                eprintln!("gstreamer video command failed: {error}");
            }
        }
    }

    fn handle_command(&mut self, command: VideoCommand) -> Result<(), GStreamerVideoError> {
        match command {
            VideoCommand::Configure { codec, codec_data } => {
                if let Some(existing) = self.pipeline.take() {
                    existing.stop();
                }
                self.pipeline = Some(GStreamerPipeline::start(
                    codec,
                    codec_data,
                    self.window_handle,
                )?);
            }
            VideoCommand::Payload(payload) => {
                let Some(pipeline) = self.pipeline.as_ref() else {
                    eprintln!("dropping video payload before codec config");
                    return Ok(());
                };
                let buffer = gst_buffer_from_slice(&payload)?;
                pipeline.appsrc.push_buffer(buffer)?;
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
            eprintln!(
                "gstreamer video packet failed for stream {}: {error}",
                self.id
            );
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
    fn handle_packet(&self, packet: VideoPacket) -> Result<(), GStreamerVideoError> {
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

    fn send_command(&self, command: VideoCommand) -> Result<(), GStreamerVideoError> {
        self.sender
            .send(command)
            .map_err(|error| GStreamerVideoError::new(error.to_string()))?;
        let _ = self.proxy.send_event(AppEvent::VideoCommandQueued);
        Ok(())
    }
}

enum VideoCommand {
    Configure { codec: Codec, codec_data: Vec<u8> },
    Payload(Vec<u8>),
}

struct GStreamerPipeline {
    appsrc: gstreamer_app::AppSrc,
    pipeline: gstreamer::Pipeline,
}

impl GStreamerPipeline {
    fn start(
        codec: Codec,
        codec_data: impl AsRef<[u8]> + Send + 'static,
        window_handle: Option<u64>,
    ) -> Result<Self, GStreamerVideoError> {
        use gstreamer as gst;
        use gstreamer::prelude::*;
        use gstreamer_video::prelude::VideoOverlayExtManual;

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
        .map_err(|_| GStreamerVideoError::new("GStreamer launch did not create a pipeline"))?;

        let encoded_caps = codec.encoded_caps(codec_data.as_ref())?;
        let appsrc = pipeline
            .by_name("airplay_src")
            .ok_or_else(|| GStreamerVideoError::new("missing AirPlay appsrc"))?
            .dynamic_cast::<gstreamer_app::AppSrc>()
            .map_err(|_| GStreamerVideoError::new("airplay_src element has unexpected type"))?;
        appsrc.set_caps(Some(&encoded_caps));

        if let Some(window_handle) = window_handle {
            let overlay = pipeline
                .by_name("video_sink")
                .ok_or_else(|| GStreamerVideoError::new("missing video sink"))?
                .dynamic_cast::<gstreamer_video::VideoOverlay>()
                .map_err(|_| {
                    GStreamerVideoError::new("glimagesink does not implement GstVideoOverlay")
                })?;
            unsafe {
                overlay.set_window_handle(window_handle as _);
            }
        }

        pipeline.set_state(gst::State::Playing)?;

        Ok(Self { appsrc, pipeline })
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
    fn parser_name(self) -> &'static str {
        match self {
            Self::H264 => "h264parse",
            Self::H265 => "h265parse",
        }
    }

    fn encoded_caps(self, codec_data: &[u8]) -> Result<gstreamer::Caps, GStreamerVideoError> {
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

fn gst_buffer_from_slice(data: &[u8]) -> Result<gstreamer::Buffer, GStreamerVideoError> {
    let mut buffer = gstreamer::Buffer::with_size(data.len())?;
    let Some(buf) = buffer.get_mut() else {
        return Err(GStreamerVideoError::new(
            "failed to get writable GStreamer buffer",
        ));
    };
    buf.copy_from_slice(0, data)
        .map_err(|_| GStreamerVideoError::new("failed to copy data into GStreamer buffer"))?;
    Ok(buffer)
}

#[derive(Debug)]
struct GStreamerVideoError {
    message: String,
}

impl GStreamerVideoError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for GStreamerVideoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for GStreamerVideoError {}

impl From<gstreamer::glib::BoolError> for GStreamerVideoError {
    fn from(error: gstreamer::glib::BoolError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<gstreamer::glib::Error> for GStreamerVideoError {
    fn from(error: gstreamer::glib::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<gstreamer::StateChangeError> for GStreamerVideoError {
    fn from(error: gstreamer::StateChangeError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<gstreamer::FlowError> for GStreamerVideoError {
    fn from(error: gstreamer::FlowError) -> Self {
        Self::new(error.to_string())
    }
}

fn init_gstreamer() -> Result<(), GStreamerVideoError> {
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
            return Err(GStreamerVideoError::new(format!(
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

#[cfg(not(target_os = "android"))]
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
    let mut app = App {
        proxy: Some(event_loop.create_proxy()),
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
