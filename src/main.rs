fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt::try_init();
    let options = main::RunOptions::from_env_and_args(std::env::args().skip(1))?;
    main::run_with_options(
        winit::event_loop::EventLoop::<main::AppEvent>::with_user_event().build()?,
        options,
    )?;
    Ok(())
}
