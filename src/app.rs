use std::{error::Error, path::PathBuf, thread::JoinHandle};

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
    replay: Option<JoinHandle<()>>,
    proxy: Option<EventLoopProxy<AppEvent>>,
    options: RunOptions,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub video_backend: VideoBackend,
    pub input: VideoInput,
}

#[derive(Debug, Clone)]
pub enum VideoInput {
    Live { capture_path: Option<PathBuf> },
    Replay { path: PathBuf },
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            video_backend: VideoBackend::Auto,
            input: VideoInput::Live { capture_path: None },
        }
    }
}

impl RunOptions {
    pub fn from_env_and_args<I, S>(args: I) -> Result<Self, Box<dyn Error>>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut options = Self {
            video_backend: match std::env::var("AIRPLAY_RS_VIDEO_BACKEND") {
                Ok(value) => VideoBackend::parse_name(&value)?,
                Err(std::env::VarError::NotPresent) => VideoBackend::Auto,
                Err(error) => return Err(Box::new(error)),
            },
            input: env_input_mode()?,
        };

        let mut args = args.into_iter().map(Into::into);
        while let Some(arg) = args.next() {
            if let Some(value) = arg.strip_prefix("--video-backend=") {
                options.video_backend = VideoBackend::parse_name(value)?;
            } else if arg == "--video-backend" {
                let Some(value) = args.next() else {
                    return Err(invalid_input("--video-backend requires a value"));
                };
                options.video_backend = VideoBackend::parse_name(&value)?;
            } else if arg == "--native-video" {
                options.video_backend = VideoBackend::Native;
            } else if arg == "--gstreamer-video" {
                options.video_backend = VideoBackend::GStreamer;
            } else if arg == "--side-by-side-video" || arg == "--dual-video" {
                options.video_backend = VideoBackend::SideBySide;
            } else if let Some(value) = arg
                .strip_prefix("--capture-video=")
                .or_else(|| arg.strip_prefix("--capture="))
            {
                set_capture_path(&mut options, PathBuf::from(value))?;
            } else if arg == "--capture-video" || arg == "--capture" {
                let Some(value) = args.next() else {
                    return Err(invalid_input("--capture-video requires a path"));
                };
                set_capture_path(&mut options, PathBuf::from(value))?;
            } else if let Some(value) = arg
                .strip_prefix("--replay-video=")
                .or_else(|| arg.strip_prefix("--replay="))
            {
                set_replay_path(&mut options, PathBuf::from(value))?;
            } else if arg == "--replay-video" || arg == "--replay" {
                let Some(value) = args.next() else {
                    return Err(invalid_input("--replay-video requires a path"));
                };
                set_replay_path(&mut options, PathBuf::from(value))?;
            } else {
                return Err(invalid_input(format!(
                    "unknown argument '{arg}'. Use --video-backend auto|native|gstreamer|side-by-side, --capture-video PATH, or --replay-video PATH"
                )));
            }
        }

        options.video_backend.ensure_available()?;
        Ok(options)
    }
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
            self.options.video_backend,
            match &self.options.input {
                VideoInput::Live { capture_path } => capture_path.as_deref(),
                VideoInput::Replay { .. } => None,
            },
        );
        match &self.options.input {
            VideoInput::Live { .. } => {
                self.airplay = Some(AirPlayServer::start(video.device()));
            }
            VideoInput::Replay { path } => {
                self.replay = Some(
                    video.start_replay(
                        path.clone(),
                        self.proxy
                            .as_ref()
                            .expect("event loop proxy must be initialized")
                            .clone(),
                    ),
                );
            }
        }
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
        self.replay = None;
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
    run_with_options(
        event_loop,
        RunOptions {
            video_backend,
            ..Default::default()
        },
    )
}

pub fn run_with_options(
    event_loop: EventLoop<AppEvent>,
    options: RunOptions,
) -> Result<(), winit::error::EventLoopError> {
    let mut app = App {
        proxy: Some(event_loop.create_proxy()),
        options,
        ..Default::default()
    };
    event_loop.run_app(&mut app)
}

fn env_input_mode() -> Result<VideoInput, Box<dyn Error>> {
    let capture_path = optional_env_path("AIRPLAY_RS_CAPTURE_PATH")?;
    let replay_path = optional_env_path("AIRPLAY_RS_REPLAY_PATH")?;

    match (capture_path, replay_path) {
        (Some(_), Some(_)) => Err(invalid_input(
            "AIRPLAY_RS_CAPTURE_PATH and AIRPLAY_RS_REPLAY_PATH cannot both be set",
        )),
        (Some(capture_path), None) => Ok(VideoInput::Live {
            capture_path: Some(capture_path),
        }),
        (None, Some(path)) => Ok(VideoInput::Replay { path }),
        (None, None) => Ok(VideoInput::Live { capture_path: None }),
    }
}

fn optional_env_path(name: &str) -> Result<Option<PathBuf>, Box<dyn Error>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(PathBuf::from(value))),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(Box::new(error)),
    }
}

fn set_capture_path(options: &mut RunOptions, path: PathBuf) -> Result<(), Box<dyn Error>> {
    if matches!(options.input, VideoInput::Replay { .. }) {
        return Err(invalid_input(
            "--capture-video and --replay-video cannot be used together",
        ));
    }
    options.input = VideoInput::Live {
        capture_path: Some(path),
    };
    Ok(())
}

fn set_replay_path(options: &mut RunOptions, path: PathBuf) -> Result<(), Box<dyn Error>> {
    if matches!(
        options.input,
        VideoInput::Live {
            capture_path: Some(_)
        }
    ) {
        return Err(invalid_input(
            "--capture-video and --replay-video cannot be used together",
        ));
    }
    options.input = VideoInput::Replay { path };
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    ))
}
