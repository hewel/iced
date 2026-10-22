//! Strict 10-bit compositor using iced's normal renderer and surface lifecycle.
use crate::{Player, Session, gpu::DeviceContext, interop::Host};
use iced::advanced::graphics::core::{Color, backend, renderer};
use iced_wgpu::{
    Engine, Renderer,
    graphics::{self, Shell, Viewport, compositor},
    wgpu,
};
use std::{rc::Rc, sync::Arc};

pub struct Compositor {
    player: Player,
    instance: wgpu::Instance,
    context: Rc<DeviceContext>,
    engine: Engine,
}

impl Compositor {
    /// Use with `iced::daemon(...).run_with_compositor(...)`.
    pub async fn create(
        player: Player,
        display: impl compositor::Display,
        window: Arc<iced_winit::winit::window::Window>,
        shell: Shell,
    ) -> Result<Self, backend::Error> {
        Self::request(player, display, window, shell)
            .await
            .map_err(|error| backend::Error::GraphicsAdapterNotFound {
                backend: "mpv Vulkan host",
                reason: backend::Reason::RequestFailed(error.to_string()),
            })
    }

    async fn request(
        player: Player,
        display: impl compositor::Display,
        window: Arc<iced_winit::winit::window::Window>,
        shell: Shell,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let existing = player
            .session
            .borrow()
            .as_ref()
            .map(|s| (s.instance.clone(), s.context.clone()));
        let (instance, context) = if let Some(existing) = existing {
            existing
        } else {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::VULKAN,
                ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display))
            });
            let surface = instance.create_surface(window.clone())?;
            let context = Rc::new(DeviceContext::new(&instance, &surface).await?);
            let capabilities = surface.get_capabilities(&context.adapter);
            if !capabilities
                .alpha_modes
                .contains(&wgpu::CompositeAlphaMode::Opaque)
            {
                return Err("opaque 10-bit surface required".into());
            }
            let size = window.inner_size();
            let mut settings = player.settings.clone();
            settings.width = size.width;
            settings.height = size.height;
            settings.validate()?;
            let wake = player.wake.clone();
            let host = Host::new(context.clone(), &settings, move || wake())?;
            let current = {
                let lock = host.queue_lock();
                let _guard = lock.lock();
                Host::private_texture(&context.device, &context.queue, size.width, size.height)
            };
            *player.session.borrow_mut() = Some(Session {
                host,
                current,
                context: context.clone(),
                instance: instance.clone(),
                started: std::time::Instant::now(),
                statistics: Default::default(),
            });
            (player.wake)();
            eprintln!(
                "mpv host {:?}: Rgb10a2Unorm BT709 gamma22 full white=203 black=.203; {}x{}; library={}",
                settings.host_abi, size.width, size.height, settings.libmpv
            );
            (instance, context)
        };
        let lock = player
            .session
            .borrow()
            .as_ref()
            .expect("initialized session")
            .host
            .queue_lock();
        let engine = Engine::new_with_queue_synchronization(
            &context.adapter,
            context.device.clone(),
            context.queue.clone(),
            wgpu::TextureFormat::Rgb10a2Unorm,
            None,
            shell,
            lock,
        );
        Ok(Self {
            player,
            instance,
            context,
            engine,
        })
    }
}

impl graphics::Compositor for Compositor {
    type Renderer = Renderer;
    type Surface = wgpu::Surface<'static>;

    async fn new(
        _settings: backend::Settings,
        _display: impl compositor::Display + Clone,
        _window: impl compositor::Window + Clone,
        _shell: Shell,
    ) -> Result<Self, backend::Error> {
        Err(backend::Error::GraphicsAdapterNotFound {
            backend: "mpv Vulkan host",
            reason: backend::Reason::RequestFailed(
                "use Compositor::create with the application's Player".into(),
            ),
        })
    }

    fn create_renderer(&self, settings: renderer::Settings) -> Renderer {
        Renderer::new(self.engine.clone(), settings)
    }

    fn create_surface(
        &mut self,
        window: impl compositor::Window + Clone,
        width: u32,
        height: u32,
    ) -> Self::Surface {
        let mut surface = self
            .instance
            .create_surface(wgpu::SurfaceTarget::Window(Box::new(window)))
            .expect("compatible native window");
        if width > 0 && height > 0 {
            self.configure_surface(&mut surface, width, height);
        }
        surface
    }

    fn configure_surface(&mut self, surface: &mut Self::Surface, width: u32, height: u32) {
        let session = self.player.session.borrow();
        let session = session.as_ref().expect("initialized session");
        {
            let lock = session.host.queue_lock();
            let _guard = lock.lock();
            surface.configure(
                &self.context.device,
                &wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: wgpu::TextureFormat::Rgb10a2Unorm,
                    color_space: wgpu::SurfaceColorSpace::Auto,
                    width,
                    height,
                    present_mode: wgpu::PresentMode::Fifo,
                    alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                    view_formats: vec![],
                    desired_maximum_frame_latency: 2,
                },
            );
        }
        if let Err(error) = session.host.resize(width, height) {
            session.host.fail(format!("mpv resize: {error}"));
        }
    }

    fn information(&self) -> compositor::Information {
        compositor::Information {
            adapter: self.context.adapter.get_info().name,
            backend: "Vulkan (shared mpv host)".into(),
        }
    }

    fn present(
        &mut self,
        renderer: &mut Renderer,
        surface: &mut Self::Surface,
        viewport: &Viewport,
        background: Color,
        on_pre_present: impl FnOnce(),
    ) -> Result<(), compositor::SurfaceError> {
        iced_wgpu::window::compositor::present(
            renderer,
            surface,
            viewport,
            background,
            on_pre_present,
        )?;
        let mut session = self.player.session.borrow_mut();
        let session = session.as_mut().expect("initialized session");
        session.statistics.presentations += 1;
        session.statistics.last_presented_at = Some(session.started.elapsed().as_secs_f64());
        Ok(())
    }

    fn screenshot(
        &mut self,
        renderer: &mut Renderer,
        viewport: &Viewport,
        background: Color,
    ) -> Vec<u8> {
        renderer.screenshot(viewport, background)
    }
}
