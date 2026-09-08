mod controls;
mod gpu;
mod interop;
mod video;

use controls::Controls;
use gpu::DeviceContext;
use iced_wgpu::{
    Engine, Renderer,
    graphics::{Shell, Viewport},
    wgpu,
};
use iced_winit::{
    conversion,
    core::{self, Event, Size, Theme, mouse, renderer, shell},
    runtime::user_interface::{self, UserInterface},
    winit,
};
use interop::Host;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use video::Video;
use winit::{
    event::WindowEvent,
    event_loop::{ControlFlow, EventLoop},
    keyboard::ModifiersState,
};

#[derive(clap::Parser, Clone)]
#[command(about = "Linux Vulkan 10-bit SDR libmpv GPU host with real iced controls")]
pub struct Options {
    /// Media file or URL.
    source: String,
    #[arg(long, default_value_t = 0.0)]
    start: f64,
    /// Requested physical window and mpv target width, not logical pixels.
    #[arg(long, default_value_t = 1280)]
    width: u32,
    #[arg(long, default_value_t = 720)]
    height: u32,
    #[arg(long)]
    pause: bool,
    /// Exit after this many wall-clock seconds (includes paused playback).
    #[arg(long)]
    duration: Option<f64>,
    #[arg(long)]
    ipc: Option<String>,
    #[arg(long, default_value = "/home/hewel/Codes/mpv/build/libmpv.so.2.5.0")]
    libmpv: String,
    #[arg(
        long,
        default_value = "/home/hewel/Codes/mpv/TOOLS/gpu-next-host-baseline.conf"
    )]
    baseline: String,
    /// Suppress only iced controls for pixel comparison, never change video settings.
    #[arg(long)]
    no_overlay: bool,
}

#[derive(Debug)]
enum Wake {
    Video,
}

// Field order retains the feature chain and VkDevice until mpv teardown has
// joined its VO thread and all host copies have completed.
struct Running {
    host: Host,
    context: DeviceContext,
    window: Arc<winit::window::Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    video: Video,
    current: wgpu::Texture,
    controls: Controls,
    cache: user_interface::Cache,
    viewport: Viewport,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    started: Instant,
    presentations: u64,
    copies: u64,
    last_presented_at: Option<f64>,
}

impl Drop for Running {
    fn drop(&mut self) {
        eprintln!(
            "SDR presentation requests={} GPU copies={} last_present_elapsed={:?}s source_pts=unavailable",
            self.presentations, self.copies, self.last_presented_at
        );
    }
}
struct App {
    options: Options,
    proxy: winit::event_loop::EventLoopProxy<Wake>,
    running: Option<Running>,
    error: Option<String>,
}
impl App {
    fn initialize(
        &self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> Result<Running, Box<dyn std::error::Error>> {
        let window = Arc::new(
            event_loop.create_window(
                winit::window::WindowAttributes::default()
                    .with_title("mpv → iced · 10-bit SDR")
                    .with_inner_size(winit::dpi::PhysicalSize::new(
                        self.options.width,
                        self.options.height,
                    )),
            )?,
        );
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Err("Window has zero physical extent".into());
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let context = futures::executor::block_on(DeviceContext::new(&instance, &surface))?;
        let caps = surface.get_capabilities(&context.adapter);
        let format = wgpu::TextureFormat::Rgb10a2Unorm;
        if !caps.formats.contains(&format) {
            return Err(format!(
                "10-bit UNORM surface is required; available: {:?}",
                caps.formats
            )
            .into());
        }
        if !caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            return Err("Opaque surface alpha is required for native SDR comparison".into());
        }
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&context.device, &config);
        let viewport = Viewport::with_physical_size(
            Size::new(size.width, size.height),
            renderer::Scale {
                window: window.scale_factor() as f32,
                application: 1.0,
            },
        );
        let engine = Engine::new(
            &context.adapter,
            context.device.clone(),
            context.queue.clone(),
            format,
            None,
            Shell::headless(),
        );
        let renderer = Renderer::new(engine, renderer::Settings::default());
        let mut video = Video::new(&context.device);
        let current =
            Host::private_texture(&context.device, &context.queue, size.width, size.height);
        video.set_texture(&context.device, &current);
        let proxy = self.proxy.clone();
        let mut actual_options = self.options.clone();
        actual_options.width = size.width;
        actual_options.height = size.height;
        let host = Host::new(&context, &actual_options, move || {
            let _ = proxy.send_event(Wake::Video);
        })?;
        eprintln!(
            "SDR source/target=Rgb10a2Unorm BT709 gamma22 full white=203 black=.203; physical={}x{} source-target={}x{}; library={}",
            size.width, size.height, size.width, size.height, self.options.libmpv
        );
        window.request_redraw();
        Ok(Running {
            host,
            context,
            window,
            surface,
            config,
            renderer,
            video,
            current,
            controls: Controls::new(),
            cache: user_interface::Cache::new(),
            viewport,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            started: Instant::now(),
            presentations: 0,
            copies: 0,
            last_presented_at: None,
        })
    }
}
impl winit::application::ApplicationHandler<Wake> for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.running.is_none() {
            match self.initialize(event_loop) {
                Ok(running) => self.running = Some(running),
                Err(error) => {
                    self.error = Some(error.to_string());
                    event_loop.exit();
                }
            }
        }
    }
    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, _event: Wake) {
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(running) = &mut self.running {
            if self
                .options
                .duration
                .is_some_and(|seconds| running.started.elapsed().as_secs_f64() >= seconds)
            {
                event_loop.exit();
                return;
            }
            // wgpu completion callbacks recycle source slots. Poll does not call
            // mpv; callbacks only take the short pool lock and enqueue a wake.
            let lock = running.host.queue_lock();
            let _guard = lock.lock();
            if let Err(error) = running.context.device.poll(wgpu::PollType::Poll) {
                self.error = Some(error.to_string());
                event_loop.exit();
            }
            drop(_guard);
            if let Err(error) = running.host.retry_if_capacity() {
                self.error = Some(error);
                event_loop.exit();
            }
            let events = running.host.poll_events();
            let changed = !events.is_empty();
            for event in events {
                if let interop::PlayerEvent::Error(error) = &event {
                    eprintln!("mpv: {error}");
                }
                if let interop::PlayerEvent::EndFile { error: Some(error) } = &event {
                    eprintln!("mpv: {error}");
                }
                if matches!(&event, interop::PlayerEvent::Shutdown) {
                    event_loop.exit();
                }
                running.controls.observe(event);
            }
            if changed && !self.options.no_overlay {
                running.window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(5),
        ));
    }
    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(running) = &mut self.running else {
            return;
        };
        let Running {
            host,
            context,
            window,
            surface,
            config,
            renderer,
            video,
            current,
            controls,
            cache,
            viewport,
            cursor,
            modifiers,
            started,
            presentations,
            copies,
            last_presented_at,
        } = running;
        let waker = shell::Waker::noop();
        let mut messages = shell::Bus::new();
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    config.width = size.width;
                    config.height = size.height;
                    *viewport = Viewport::with_physical_size(
                        Size::new(size.width, size.height),
                        renderer::Scale {
                            window: window.scale_factor() as f32,
                            application: 1.0,
                        },
                    );
                    let lock = host.queue_lock();
                    let _guard = lock.lock();
                    surface.configure(&context.device, config);
                    drop(_guard);
                    if let Err(error) = host.resize(size.width, size.height) {
                        self.error = Some(error);
                        event_loop.exit();
                    }
                }
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = window.inner_size();
                *viewport = Viewport::with_physical_size(
                    Size::new(size.width, size.height),
                    renderer::Scale {
                        window: window.scale_factor() as f32,
                        application: 1.0,
                    },
                );
                window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                *cursor = mouse::Cursor::Available(conversion::cursor_position(
                    *position,
                    viewport.scale_factor(),
                ));
            }
            WindowEvent::CursorLeft { .. } => {
                *cursor = mouse::Cursor::Unavailable;
            }
            WindowEvent::ModifiersChanged(value) => {
                *modifiers = value.state();
            }
            WindowEvent::RedrawRequested => {
                let size = window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }
                // This is the only runtime submission domain: native mpv queue
                // callbacks share this same mutex. Includes implicit renderer
                // uploads/submits, present, reconfigure and GPU copy submission.
                // There are no background image loaders in this example.
                let lock = host.queue_lock();
                let _guard = lock.lock();
                if let Some(resized) = host.copy_ready(&context.queue, current) {
                    *copies += 1;
                    if resized {
                        video.set_texture(&context.device, current);
                    }
                }
                controls.set_statistics(*presentations, *copies, *last_presented_at);
                match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                        let view = frame
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        let mut encoder = context.device.create_command_encoder(
                            &wgpu::CommandEncoderDescriptor {
                                label: Some("SDR video background"),
                            },
                        );
                        video.draw(&mut encoder, &view);
                        context.queue.submit([encoder.finish()]);
                        if !self.options.no_overlay {
                            let mut interface = UserInterface::build(
                                controls.view(),
                                viewport.logical_size(),
                                std::mem::take(cache),
                                renderer,
                            );
                            let (state, _) = interface.update(
                                window,
                                &waker,
                                &[Event::Window(core::window::Event::RedrawRequested(
                                    core::time::Instant::now(),
                                ))],
                                *cursor,
                                renderer,
                                &mut messages,
                            );
                            if let user_interface::State::Updated {
                                mouse_interaction, ..
                            } = state
                            {
                                if let Some(icon) = conversion::mouse_interaction(mouse_interaction)
                                {
                                    window.set_cursor(icon);
                                    window.set_cursor_visible(true);
                                } else {
                                    window.set_cursor_visible(false);
                                }
                            }
                            interface.draw(
                                renderer,
                                &Theme::Dark,
                                &renderer::Style::default(),
                                *cursor,
                            );
                            *cache = interface.into_cache();
                            renderer.present(None, config.format, &view, viewport);
                        }
                        window.pre_present_notify();
                        frame.present();
                        // Counts successful surface-present calls, not scanout
                        // acknowledgements; this ABI exposes no per-image PTS.
                        *presentations += 1;
                        *last_presented_at = Some(started.elapsed().as_secs_f64());
                        controls.set_statistics(*presentations, *copies, *last_presented_at);
                    }
                    wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(&context.device, config);
                        window.request_redraw();
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        self.error = Some("Vulkan surface validation error".into());
                        event_loop.exit();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        if !self.options.no_overlay
            && let Some(event) =
                conversion::window_event(event, window.scale_factor() as f32, *modifiers)
        {
            let lock = host.queue_lock();
            let _guard = lock.lock();
            let mut interface = UserInterface::build(
                controls.view(),
                viewport.logical_size(),
                std::mem::take(cache),
                renderer,
            );
            let _ = interface.update(window, &waker, &[event], *cursor, renderer, &mut messages);
            *cache = interface.into_cache();
            window.request_redraw();
        }
        // All queue guards above are out of scope. mpv commands may interact
        // with its VO thread and must never run while holding the shared lock.
        for message in messages {
            if let Some(command) = controls.update(message)
                && let Err(error) = host.execute(command)
            {
                eprintln!("mpv command: {error}");
                controls.report_error(error);
            }
            window.request_redraw();
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    let options = Options::parse();
    if options.width == 0
        || options.height == 0
        || options.width > i32::MAX as u32
        || options.height > i32::MAX as u32
        || !options.start.is_finite()
        || options.start < 0.0
        || options.duration.is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        return Err(
            "positive physical dimensions/duration and nonnegative finite start required".into(),
        );
    }
    tracing_subscriber::fmt::init();
    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    let mut app = App {
        options,
        proxy: event_loop.create_proxy(),
        running: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    // Drop host before graphics context, without holding the shared queue lock.
    app.running.take();
    if let Some(error) = app.error {
        return Err(error.into());
    }
    Ok(())
}
