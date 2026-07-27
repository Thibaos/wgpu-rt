use std::{collections::HashSet, sync::Arc, time::Instant};

use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey, SmolStr},
    window::{CursorGrabMode, Window, WindowId},
};

use crate::{app::App, utils::get_adapter_with_capabilities_or_from_env};

fn init_logger() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();
}

struct SurfaceWrapper {
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
}

impl SurfaceWrapper {
    fn new() -> Self {
        Self {
            surface: None,
            config: None,
        }
    }

    fn resize(&mut self, context: &RenderContext, size: PhysicalSize<u32>) {
        log::info!("Surface resize {size:?}");

        let config = self.config.as_mut().unwrap();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        let surface = self.surface.as_ref().unwrap();
        surface.configure(&context.device, config);
    }

    fn acquire(&mut self, context: &RenderContext) -> Option<wgpu::SurfaceTexture> {
        let surface = self.surface.as_ref().unwrap();
        let config = self.config();

        for attempt in 0..3 {
            match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => return Some(frame),
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    log::warn!("Surface acquire returned suboptimal frame");
                    return Some(frame);
                }
                wgpu::CurrentSurfaceTexture::Timeout => {
                    log::warn!(
                        "Surface acquire timed out (attempt {}/3), retrying...",
                        attempt + 1
                    );
                    std::thread::yield_now();
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    log::info!("Surface outdated, reconfiguring...");
                    surface.configure(&context.device, config);
                    break;
                }
                _ => {
                    log::error!(
                        "Surface acquire failed (attempt {}/3), retrying...",
                        attempt + 1
                    );
                    std::thread::yield_now();
                }
            }
        }

        surface.configure(&context.device, config);
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            _ => {
                log::error!("Failed to acquire surface texture after reconfiguration");
                None
            }
        }
    }

    fn suspend(&mut self) {
        if cfg!(target_os = "android") {
            self.surface = None;
        }
    }

    fn config(&self) -> &wgpu::SurfaceConfiguration {
        self.config.as_ref().unwrap()
    }
}

struct RenderContext {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl RenderContext {
    async fn init(
        display_handle: winit::event_loop::OwnedDisplayHandle,
        window: Arc<Window>,
    ) -> (Self, SurfaceWrapper) {
        log::info!("Initializing wgpu...");

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display_handle),
        ));

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = get_adapter_with_capabilities_or_from_env(
            &instance,
            &App::required_features(),
            &App::required_downlevel_capabilities(),
            &Some(&surface),
        )
        .await;

        let mut needed_limits = App::required_limits().using_resolution(adapter.limits());
        needed_limits.max_binding_array_elements_per_shader_stage =
            crate::world::chunk::TOTAL_CHUNKS
                .min(adapter.limits().max_binding_array_elements_per_shader_stage);

        let info = adapter.get_info();
        log::info!("Selected adapter: {} ({:?})", info.name, info.backend);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: (App::optional_features() & adapter.features())
                    | App::required_features(),
                required_limits: needed_limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Unable to find a suitable GPU adapter!");

        let window_size = window.inner_size();
        let width = window_size.width.max(1);
        let height = window_size.height.max(1);

        log::info!("Surface resume {window_size:?}");

        let mut config = surface
            .get_default_config(&adapter, width, height)
            .expect("Surface isn't supported by the adapter.");

        if App::SRGB {
            let view_format = config.format.add_srgb_suffix();
            config.view_formats.push(view_format);
        } else {
            let format = config.format.remove_srgb_suffix();
            config.format = format;
            config.view_formats.push(format);
        }
        config.desired_maximum_frame_latency = 3;

        surface.configure(&device, &config);

        let surface_wrapper = SurfaceWrapper {
            surface: Some(surface),
            config: Some(config),
        };

        (
            Self {
                adapter,
                device,
                queue,
            },
            surface_wrapper,
        )
    }
}

struct FrameCounter {
    last_printed_instant: Instant,
    frame_count: u32,
}

impl FrameCounter {
    fn new() -> Self {
        Self {
            last_printed_instant: Instant::now(),
            frame_count: 0,
        }
    }

    fn update(&mut self) {
        self.frame_count += 1;
        let new_instant = Instant::now();
        let elapsed_secs = (new_instant - self.last_printed_instant).as_secs_f32();
        if elapsed_secs > 1.0 {
            let elapsed_ms = elapsed_secs * 1000.0;
            let frame_time = elapsed_ms / self.frame_count as f32;
            let fps = self.frame_count as f32 / elapsed_secs;
            log::info!("Frame time {frame_time:.2}ms ({fps:.1} FPS)");

            self.last_printed_instant = new_instant;
            self.frame_count = 0;
        }
    }
}

struct Framework {
    window: Option<Arc<Window>>,
    surface: SurfaceWrapper,
    context: Option<RenderContext>,
    app: Option<App>,
    frame_counter: FrameCounter,
    pressed_keys: HashSet<Key<SmolStr>>,
    cursor_grab_mode: CursorGrabMode,
}

impl Framework {
    fn new() -> Self {
        Self {
            window: None,
            surface: SurfaceWrapper::new(),
            context: None,
            app: None,
            frame_counter: FrameCounter::new(),
            pressed_keys: HashSet::new(),
            cursor_grab_mode: CursorGrabMode::None,
        }
    }

    fn try_set_cursor_grab(window: &Window, desired: CursorGrabMode) -> CursorGrabMode {
        match window.set_cursor_grab(desired) {
            Ok(()) => desired,
            Err(e) => {
                log::warn!("set_cursor_grab({desired:?}) failed: {e}; trying Locked");
                match window.set_cursor_grab(CursorGrabMode::Locked) {
                    Ok(()) => CursorGrabMode::Locked,
                    Err(e2) => {
                        log::warn!("set_cursor_grab(Locked) also failed: {e2}; grab disabled");
                        CursorGrabMode::None
                    }
                }
            }
        }
    }
}

impl ApplicationHandler for Framework {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Ray Cube")
                        .with_inner_size(PhysicalSize::new(1920, 1080)),
                )
                .unwrap(),
        );

        let (context, surface) = pollster::block_on(RenderContext::init(
            event_loop.owned_display_handle(),
            window.clone(),
        ));

        let app = App::init(
            surface.config(),
            &context.adapter,
            &context.device,
            &context.queue,
        );

        self.window = Some(window);
        self.surface = surface;
        self.context = Some(context);
        self.app = Some(app);

        self.window.as_ref().unwrap().request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.surface.suspend();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let window = self.window.as_ref().unwrap();
        let context = self.context.as_ref().unwrap();
        let app = self.app.as_mut().unwrap();

        match event {
            WindowEvent::Resized(size) => {
                self.surface.resize(context, size);
                app.resize(self.surface.config(), &context.device, &context.queue);
                window.request_redraw();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        physical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                if let Key::Named(NamedKey::Escape) = &logical_key
                    && !self.pressed_keys.contains(&Key::Named(NamedKey::Escape))
                {
                    let desired = match self.cursor_grab_mode {
                        CursorGrabMode::None => CursorGrabMode::Confined,
                        _ => CursorGrabMode::None,
                    };
                    let applied = Self::try_set_cursor_grab(window, desired);
                    self.cursor_grab_mode = applied;
                    window.set_cursor_visible(self.cursor_grab_mode == CursorGrabMode::None);
                }

                if physical_key
                    == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyH)
                {
                    app.toggle_heatmap();
                }
                self.pressed_keys.insert(logical_key);
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Released,
                        ..
                    },
                ..
            } => {
                self.pressed_keys.remove(&logical_key);
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                self.frame_counter.update();

                if let Some(frame) = self.surface.acquire(context) {
                    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                        format: Some(self.surface.config().view_formats[0]),
                        ..Default::default()
                    });

                    app.render(&view, &context.device, &context.queue, &self.pressed_keys);

                    window.pre_present_notify();
                    context.queue.present(frame);
                }

                window.request_redraw();
            }
            other => {
                app.update(other);
            }
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let (Some(_window), Some(app)) = (self.window.as_ref(), self.app.as_mut()) {
            match event {
                DeviceEvent::MouseMotion { delta } => {
                    if self.cursor_grab_mode == CursorGrabMode::Confined {
                        app.update_look_position(delta);
                    }
                }
                DeviceEvent::MouseWheel {
                    delta: MouseScrollDelta::LineDelta(_, y_delta),
                } => {
                    app.player_controller.handle_speed_change(y_delta);
                }
                _ => {}
            }
        }
    }
}

pub fn run() {
    init_logger();

    log::debug!(
        "Enabled backends: {:?}",
        wgpu::Instance::enabled_backend_features()
    );

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut framework = Framework::new();
    log::info!("Entering event loop...");
    event_loop.run_app(&mut framework).unwrap();
}
