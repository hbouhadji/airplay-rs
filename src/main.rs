fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt::try_init();
    main::run(winit::event_loop::EventLoop::<main::AppEvent>::with_user_event().build()?)?;
    Ok(())
}
