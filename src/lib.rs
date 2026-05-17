use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::EventLoop,
    window::{Window, WindowAttributes},
};

#[derive(Default)]
struct App {
    window: Option<Window>,
    video: Option<VideoSource>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("winit + gstreamer native sink")
                    .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0)),
            )
            .expect("failed to create window");

        self.video = Some(VideoSource::start(&window));
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
                self.video = None;
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(video) = self.video.as_ref() {
                    video.resize(size);
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(target_os = "android"))]
struct VideoSource {
    pipeline: gstreamer::Pipeline,
    overlay: gstreamer_video::VideoOverlay,
}

#[cfg(not(target_os = "android"))]
impl VideoSource {
    fn start(window: &Window) -> Self {
        use gstreamer as gst;
        use gstreamer::prelude::*;
        use gstreamer_video::prelude::*;

        gst::init().expect("failed to initialize GStreamer");

        let handle = native_window_handle(window);
        let pipeline = gst::parse::launch(
            "gltestsrc pattern=smpte is-live=true \
             ! glimagesink name=video_sink force-aspect-ratio=true",
        )
        .expect("failed to create GStreamer pipeline")
        .downcast::<gst::Pipeline>()
        .expect("GStreamer launch did not create a pipeline");

        let sink = pipeline.by_name("video_sink").expect("missing video sink");
        let overlay = sink
            .dynamic_cast::<gstreamer_video::VideoOverlay>()
            .expect("video sink does not implement GstVideoOverlay");

        unsafe {
            overlay.set_window_handle(handle);
        }
        overlay.handle_events(true);
        set_render_rectangle(&overlay, window.inner_size());

        pipeline
            .set_state(gst::State::Playing)
            .expect("failed to start GStreamer pipeline");

        Self { pipeline, overlay }
    }

    fn resize(&self, size: PhysicalSize<u32>) {
        use gstreamer_video::prelude::VideoOverlayExt;

        set_render_rectangle(&self.overlay, size);
        self.overlay.expose();
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for VideoSource {
    fn drop(&mut self) {
        use gstreamer as gst;
        use gstreamer::prelude::*;

        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(not(target_os = "android"))]
fn set_render_rectangle(overlay: &gstreamer_video::VideoOverlay, size: PhysicalSize<u32>) {
    use gstreamer_video::prelude::*;

    if size.width == 0 || size.height == 0 {
        return;
    }

    let _ = overlay.set_render_rectangle(0, 0, size.width as i32, size.height as i32);
}

#[cfg(not(target_os = "android"))]
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
        other => panic!("unsupported native window handle for GstVideoOverlay: {other:?}"),
    }
}

#[cfg(target_os = "android")]
struct VideoSource;

#[cfg(target_os = "android")]
impl VideoSource {
    fn start(_window: &Window) -> Self {
        Self
    }

    fn resize(&self, _size: PhysicalSize<u32>) {}
}

pub fn run(event_loop: EventLoop<()>) -> Result<(), winit::error::EventLoopError> {
    let mut app = App::default();
    event_loop.run_app(&mut app)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    let event_loop = EventLoop::builder()
        .with_android_app(app)
        .build()
        .expect("failed to create Android event loop");

    run(event_loop).expect("Android event loop failed");
}
