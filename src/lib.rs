use winit::{
    application::ApplicationHandler,
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
                    .with_title("airplay-rs")
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
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(video) = self.video.as_ref() {
                        video.expose();
                    }
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(video) = self.video.as_ref() {
                    video.expose();
                }
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(video) = self.video.take() {
            video.stop();
        }
        self.window = None;
    }
}

struct VideoSource {
    pipeline: gstreamer::Pipeline,
}

impl VideoSource {
    fn start(window: &Window) -> Self {
        use gstreamer as gst;
        use gstreamer::prelude::*;
        use gstreamer_video::prelude::*;

        gst::init().expect("failed to initialize GStreamer");
        register_android_gstreamer_plugins();

        let handle = native_window_handle(window);
        let pipeline = gst::parse::launch(
            "gltestsrc pattern=smpte is-live=true \
             ! glimagesink name=video_sink force-aspect-ratio=true",
        )
        .expect("failed to create GStreamer pipeline")
        .downcast::<gst::Pipeline>()
        .expect("GStreamer launch did not create a pipeline");

        let overlay = video_overlay(&pipeline);
        unsafe {
            overlay.set_window_handle(handle);
        }
        overlay.handle_events(true);

        pipeline
            .set_state(gst::State::Playing)
            .expect("failed to start GStreamer pipeline");

        Self { pipeline }
    }

    fn expose(&self) {
        use gstreamer_video::prelude::VideoOverlayExt;

        video_overlay(&self.pipeline).expose();
    }

    fn stop(self) {
        use gstreamer as gst;
        use gstreamer::prelude::*;

        let this = std::mem::ManuallyDrop::new(self);
        let _ = this.pipeline.set_state(gst::State::Null);
        let _ = this.pipeline.state(gst::ClockTime::from_seconds(2));
        // Avoid a macOS glimagesink crash in the final GObject unref during app shutdown.
        std::mem::forget(this);
    }
}

fn video_overlay(pipeline: &gstreamer::Pipeline) -> gstreamer_video::VideoOverlay {
    use gstreamer::prelude::*;

    pipeline
        .by_name("video_sink")
        .expect("missing video sink")
        .dynamic_cast::<gstreamer_video::VideoOverlay>()
        .expect("video sink does not implement GstVideoOverlay")
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

impl Drop for VideoSource {
    fn drop(&mut self) {
        use gstreamer as gst;
        use gstreamer::prelude::*;

        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = self.pipeline.state(gst::ClockTime::from_seconds(2));
    }
}

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
