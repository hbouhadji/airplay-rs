mod airplay;
mod app;
mod discovery;
mod video;

#[doc(hidden)]
pub enum AppEvent {
    VideoCommandQueued,
}

pub use app::{run, run_with_video_backend};
pub use video::VideoBackend;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    let event_loop = winit::event_loop::EventLoop::<AppEvent>::with_user_event()
        .with_android_app(app)
        .build()
        .expect("failed to create Android event loop");

    run(event_loop).expect("Android event loop failed");
}
