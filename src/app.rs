use std::sync::Arc;

use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

use crate::state::State;

#[derive(Default)]
pub struct Application<'a> {
    state: Option<State<'a>>,
    close_requested: bool,
    window: Option<Arc<Window>>,
}

impl ApplicationHandler for Application<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes =
            Window::default_attributes().with_inner_size(PhysicalSize::new(1920, 1080));

        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
        let state = pollster::block_on(State::new(window.clone()));

        self.state = Some(state);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.close_requested = true;
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                if let Key::Named(NamedKey::Escape) = key.as_ref() {
                    self.close_requested = true;
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(state) = self.state.as_mut() {
                    state.render().unwrap();
                }
            }
            _ => (),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.close_requested {
            self.window.as_ref().unwrap().request_redraw();
        }

        if self.close_requested {
            event_loop.exit();
        }
    }
}
