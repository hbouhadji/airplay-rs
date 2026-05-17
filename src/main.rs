fn main() -> Result<(), Box<dyn std::error::Error>> {
    main::run(winit::event_loop::EventLoop::new()?)?;
    Ok(())
}
