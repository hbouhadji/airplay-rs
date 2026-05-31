mod backend;
mod codec;

#[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
mod gstreamer;
#[cfg(target_os = "macos")]
mod macos;

use std::{
    convert::Infallible,
    error::Error,
    fmt,
    sync::{
        Weak,
        mpsc::{self, Receiver, Sender},
    },
};

use rairplay::playback::{
    ChannelHandle, Device, Stream,
    video::{PacketKind, VideoDevice, VideoPacket, VideoParams},
};
use winit::{event_loop::EventLoopProxy, window::Window};

pub use backend::VideoBackend;
use codec::Codec;

use crate::AppEvent;

pub(crate) struct VideoSource {
    device: AirPlayVideoDevice,
    receiver: Receiver<VideoCommand>,
    pipeline: Option<VideoPipeline>,
    window_handle: Option<u64>,
    video_backend: VideoBackend,
}

impl VideoSource {
    pub(crate) fn start(
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

    pub(crate) fn device(&self) -> AirPlayVideoDevice {
        self.device.clone()
    }

    pub(crate) fn drain_commands(&mut self) {
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

    pub(crate) fn stop(mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.stop();
        }
    }
}

#[derive(Clone)]
pub(crate) struct AirPlayVideoDevice {
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

pub(crate) struct AirPlayVideoStream {
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
    Native(macos::MacOsVideoPipeline),
    #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
    GStreamer(gstreamer::GStreamerPipeline),
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
                    Ok(Self::Native(macos::MacOsVideoPipeline::start(
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
                    Ok(Self::GStreamer(gstreamer::GStreamerPipeline::start(
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

#[derive(Debug)]
pub(super) struct VideoError {
    message: String,
}

impl VideoError {
    pub(super) fn new(message: impl Into<String>) -> Self {
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
        other => panic!("unsupported native window handle for video overlay: {other:?}"),
    }
}
