use std::error::Error;

use app::Application;
use winit::event_loop::EventLoop;

mod acceleration_structure;
mod app;
mod camera;
mod compute;
mod state;
mod texture;

fn main() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;

    let mut app = Application::default();

    event_loop.run_app(&mut app)?;

    Ok(())
}
