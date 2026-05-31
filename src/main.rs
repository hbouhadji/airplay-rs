fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt::try_init();
    let video_backend = main::VideoBackend::from_env_and_args(std::env::args().skip(1))?;
    main::run_with_video_backend(
        winit::event_loop::EventLoop::<main::AppEvent>::with_user_event().build()?,
        video_backend,
    )?;
    Ok(())
}
