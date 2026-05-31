use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes},
};

use crate::{
    AppEvent,
    airplay::AirPlayServer,
    video::{VideoBackend, VideoSource},
};

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
