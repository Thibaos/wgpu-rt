use std::{collections::HashSet, sync::Arc, time::Instant};

use wgpu::Surface;
use winit::{
    dpi::PhysicalSize,
    event::{
        DeviceEvent, ElementState, Event, KeyEvent, MouseScrollDelta, StartCause, WindowEvent,
    },
    event_loop::{EventLoop, EventLoopWindowTarget},
    keyboard::{Key, NamedKey, SmolStr},
    window::{CursorGrabMode, Window},
};

// Initialize logging in platform dependant ways.
fn init_logger() {
    // parse_default_env will read the RUST_LOG environment variable and apply it on top
    // of these default filters.
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();
}

struct EventLoopWrapper {
    event_loop: EventLoop<()>,
    window: Arc<Window>,
    pressed_keys: HashSet<Key<SmolStr>>,
    cursor_grab_mode: CursorGrabMode,
}

impl EventLoopWrapper {
    pub fn new(title: &str) -> Self {
        let event_loop = EventLoop::new().unwrap();
        let mut builder = winit::window::WindowBuilder::new();

        builder = builder
            .with_title(title)
            .with_inner_size(PhysicalSize::new(1920, 1080));

        let window = Arc::new(builder.build(&event_loop).unwrap());

        Self {
            event_loop,
            window,
            pressed_keys: HashSet::new(),
            cursor_grab_mode: CursorGrabMode::None,
        }
    }
}

/// Wrapper type which manages the surface and surface configuration.
///
/// As surface usage varies per platform, wrapping this up cleans up the event loop code.
struct SurfaceWrapper {
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
}

impl SurfaceWrapper {
    /// Create a new surface wrapper with no surface or configuration.
    fn new() -> Self {
        Self {
            surface: None,
            config: None,
        }
    }

    /// Check if the event is the start condition for the surface.
    fn start_condition(e: &Event<()>) -> bool {
        match e {
            // On all other platforms, we can create the surface immediately.
            Event::NewEvents(StartCause::Init) => !cfg!(target_os = "android"),
            // On android we need to wait for a resumed event to create the surface.
            Event::Resumed => cfg!(target_os = "android"),
            _ => false,
        }
    }

    /// Called when an event which matches [`Self::start_condition`] is received.
    ///
    /// On all native platforms, this is where we create the surface.
    ///
    /// Additionally, we configure the surface based on the (now valid) window size.
    fn resume(&mut self, context: &RenderContext, window: Arc<Window>, srgb: bool) {
        // Window size is only actually valid after we enter the event loop.
        let window_size = window.inner_size();
        let width = window_size.width.max(1);
        let height = window_size.height.max(1);

        log::info!("Surface resume {window_size:?}");

        self.surface = Some(context.instance.create_surface(window).unwrap());

        // From here on, self.surface should be Some.

        let surface = self.surface.as_ref().unwrap();

        // Get the default configuration,
        let mut config = surface
            .get_default_config(&context.adapter, width, height)
            .expect("Surface isn't supported by the adapter.");
        if srgb {
            // Not all platforms (WebGPU) support sRGB swapchains, so we need to use view formats
            let view_format = config.format.add_srgb_suffix();
            config.view_formats.push(view_format);
        } else {
            // All platforms support non-sRGB swapchains, so we can just use the format directly.
            let format = config.format.remove_srgb_suffix();
            config.format = format;
            config.view_formats.push(format);
        };
        config.desired_maximum_frame_latency = 3;

        surface.configure(&context.device, &config);
        self.config = Some(config);
    }

    /// Resize the surface, making sure to not resize to zero.
    fn resize(&mut self, context: &RenderContext, size: PhysicalSize<u32>) {
        log::info!("Surface resize {size:?}");

        let config = self.config.as_mut().unwrap();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        let surface = self.surface.as_ref().unwrap();
        surface.configure(&context.device, config);
    }

    /// Acquire the next surface texture.
    fn acquire(&mut self, context: &RenderContext) -> wgpu::SurfaceTexture {
        let surface = self.surface.as_ref().unwrap();

        match surface.get_current_texture() {
            Ok(frame) => frame,
            // If we timed out, just try again
            Err(wgpu::SurfaceError::Timeout) => surface
                .get_current_texture()
                .expect("Failed to acquire next surface texture!"),
            Err(
                // If the surface is outdated, or was lost, reconfigure it.
                wgpu::SurfaceError::Outdated
                | wgpu::SurfaceError::Lost
                | wgpu::SurfaceError::Other
                // If OutOfMemory happens, reconfiguring may not help, but we might as well try
                | wgpu::SurfaceError::OutOfMemory,
            ) => {
                surface.configure(&context.device, self.config());
                surface
                    .get_current_texture()
                    .expect("Failed to acquire next surface texture!")
            }
        }
    }

    /// On suspend on android, we drop the surface, as it's no longer valid.
    ///
    /// A suspend event is always followed by at least one resume event.
    fn suspend(&mut self) {
        if cfg!(target_os = "android") {
            self.surface = None;
        }
    }

    fn get(&self) -> Option<&'_ Surface<'static>> {
        self.surface.as_ref()
    }

    fn config(&self) -> &wgpu::SurfaceConfiguration {
        self.config.as_ref().unwrap()
    }
}

/// Context containing global wgpu resources.
struct RenderContext {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl RenderContext {
    async fn init_async(surface: &mut SurfaceWrapper) -> Self {
        log::info!("Initializing wgpu...");

        let instance_descriptor = wgpu::InstanceDescriptor::from_env_or_default();
        let instance = wgpu::Instance::new(&instance_descriptor);
        let adapter = get_adapter_with_capabilities_or_from_env(
            &instance,
            &App::required_features(),
            &App::required_downlevel_capabilities(),
            &surface.get(),
        )
        .await;
        // Make sure we use the texture resolution limits from the adapter, so we can support images the size of the surface.
        let needed_limits = App::required_limits().using_resolution(adapter.limits());

        let info = adapter.get_info();
        log::info!("Selected adapter: {} ({:?})", info.name, info.backend);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: (App::optional_features() & adapter.features())
                    | App::required_features(),
                required_limits: needed_limits,
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Unable to find a suitable GPU adapter!");

        Self {
            instance,
            adapter,
            device,
            queue,
        }
    }
}

struct FrameCounter {
    // Instant of the last time we printed the frame time.
    last_printed_instant: Instant,
    // Number of frames since the last time we printed the frame time.
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

async fn start() {
    init_logger();

    log::debug!(
        "Enabled backends: {:?}",
        wgpu::Instance::enabled_backend_features()
    );

    let mut window_loop = EventLoopWrapper::new("Ray Cube");
    let mut surface = SurfaceWrapper::new();
    let context = RenderContext::init_async(&mut surface).await;
    let mut frame_counter = FrameCounter::new();

    let mut app = None;

    let event_loop_function = EventLoop::run;

    log::info!("Entering event loop...");
    let _ = (event_loop_function)(
        window_loop.event_loop,
        move |event: Event<()>, target: &EventLoopWindowTarget<()>| {
            match event {
                ref e if SurfaceWrapper::start_condition(e) => {
                    surface.resume(&context, window_loop.window.clone(), App::SRGB);

                    if app.is_none() {
                        app = Some(App::init(
                            surface.config(),
                            &context.adapter,
                            &context.device,
                        ));
                    }
                }
                Event::Suspended => {
                    surface.suspend();
                }
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::Resized(size) => {
                        surface.resize(&context, size);
                        app.as_mut().unwrap().resize(
                            surface.config(),
                            &context.device,
                            &context.queue,
                        );

                        window_loop.window.request_redraw();
                    }
                    WindowEvent::KeyboardInput {
                        event:
                            KeyEvent {
                                logical_key,
                                state: ElementState::Pressed,
                                ..
                            },
                        ..
                    } => {
                        if let Key::Named(named_key) = logical_key
                            && named_key == NamedKey::Escape
                            && !window_loop
                                .pressed_keys
                                .contains(&Key::Named(NamedKey::Escape))
                        {
                            match window_loop.cursor_grab_mode {
                                CursorGrabMode::None => {
                                    let new_grab_mode = CursorGrabMode::Confined;
                                    window_loop.cursor_grab_mode = new_grab_mode;
                                    window_loop.window.set_cursor_grab(new_grab_mode).unwrap();
                                    window_loop.window.set_cursor_visible(false);
                                }
                                _ => {
                                    let new_grab_mode = CursorGrabMode::None;
                                    window_loop.cursor_grab_mode = new_grab_mode;
                                    window_loop.window.set_cursor_grab(new_grab_mode).unwrap();
                                    window_loop.window.set_cursor_visible(true);
                                }
                            }
                        }

                        window_loop.pressed_keys.insert(logical_key);
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
                        window_loop.pressed_keys.remove(&logical_key);
                    }
                    WindowEvent::CloseRequested => {
                        target.exit();
                    }
                    WindowEvent::RedrawRequested => {
                        // On MacOS, currently redraw requested comes in _before_ Init does.
                        // If this happens, just drop the requested redraw on the floor.
                        //
                        // See https://github.com/rust-windowing/winit/issues/3235 for some discussion
                        if app.is_none() {
                            return;
                        }

                        frame_counter.update();

                        let frame = surface.acquire(&context);
                        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                            format: Some(surface.config().view_formats[0]),
                            ..wgpu::TextureViewDescriptor::default()
                        });

                        app.as_mut().unwrap().render(
                            &view,
                            &context.device,
                            &context.queue,
                            &window_loop.pressed_keys,
                        );

                        window_loop.window.pre_present_notify();
                        frame.present();

                        window_loop.window.request_redraw();
                    }
                    _ => app.as_mut().unwrap().update(event),
                },
                Event::DeviceEvent {
                    device_id: _,
                    event,
                } => {
                    if let Some(app) = app.as_mut() {
                        match event {
                            DeviceEvent::MouseMotion { delta } => {
                                if window_loop.cursor_grab_mode == CursorGrabMode::Confined {
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
                _ => {}
            }
        },
    );
}

pub fn run() {
    pollster::block_on(start());
}

use crate::{app::App, utils::get_adapter_with_capabilities_or_from_env};
