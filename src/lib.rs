use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes},
};

#[derive(Default)]
struct App {
    window: Option<Window>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("winit + wgpu + gstreamer")
                    .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0)),
            )
            .expect("failed to create window");

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                println!("Window resized: {}x{}", size.width, size.height);
            }
            _ => {}
        }
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
