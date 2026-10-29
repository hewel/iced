//! A shared-device mpv host with an ordinary iced widget and compositor.
//!
//! [`Player`] owns the native session, bounded image pool, and completion worker.
//! Run its notification stream as an iced task and call [`Player::drain`] on each
//! notification. All player methods run on the application thread; GPU waits and
//! native callbacks are internal. Drop the player after the window runtime exits.
mod completion;
pub mod compositor;
mod deadline;
mod gpu;
mod interop;
mod pool;
mod video;

use iced_wgpu::wgpu;
pub use interop::{Command, PlayerEvent};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
#[repr(u32)]
pub enum HostAbi {
    #[default]
    V1 = 1,
    V2 = 2,
    V3 = 3,
}

/// Playback configuration. V3 is an explicit opt-in, requiring a matching mpv.
#[derive(Debug, Clone, clap::Args)]
pub struct Settings {
    /// Media file or URL.
    pub source: String,
    #[arg(long, default_value_t = 0.0)]
    pub start: f64,
    /// Initial window width in logical pixels; targets use the actual physical extent.
    #[arg(long, default_value_t = 1280)]
    pub width: u32,
    /// Initial window height in logical pixels.
    #[arg(long, default_value_t = 720)]
    pub height: u32,
    #[arg(long)]
    pub pause: bool,
    #[arg(long)]
    pub ipc: Option<String>,
    #[arg(long, default_value = "/home/hewel/Codes/mpv/build/libmpv.so.2.5.0")]
    pub libmpv: String,
    #[arg(
        long,
        default_value = "/home/hewel/Codes/mpv/TOOLS/gpu-next-host-baseline.conf"
    )]
    pub baseline: String,
    #[arg(long, value_enum, default_value_t = HostAbi::V1)]
    pub host_abi: HostAbi,
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > i32::MAX as u32
            || self.height > i32::MAX as u32
            || !self.start.is_finite()
            || self.start < 0.0
        {
            return Err(
                "positive physical dimensions and nonnegative finite start required".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Statistics {
    /// Successful surface-present calls, not scanout acknowledgements.
    pub presentations: u64,
    /// Authorized GPU copy submissions, not presentation acknowledgements.
    pub copies: u64,
    pub last_presented_at: Option<f64>,
}

struct Session {
    // Host drops first: joins mpv and drains copies before releasing the context.
    host: interop::Host,
    context: Rc<gpu::DeviceContext>,
    instance: wgpu::Instance,
    current: wgpu::Texture,
    started: std::time::Instant,
    statistics: Statistics,
}

/// Application-thread handle. Clones share one native session, including across
/// compositor recreation. Native pointers and synchronization never escape it.
#[derive(Clone)]
pub struct Player {
    settings: Settings,
    session: Rc<RefCell<Option<Session>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Player {
    pub fn new(settings: Settings) -> Result<(Self, futures::channel::mpsc::Receiver<()>), String> {
        settings.validate()?;
        let (sender, receiver) = futures::channel::mpsc::channel(1);
        let sender = parking_lot::Mutex::new(sender);
        let wake = Arc::new(move || {
            let _ = sender.lock().try_send(());
        });
        Ok((
            Self {
                settings,
                session: Rc::new(RefCell::new(None)),
                wake,
            },
            receiver,
        ))
    }

    /// Drains notifications and schedules GPU copies of authorized frames.
    /// No fixed timer is needed, including while paused or the pool is exhausted.
    pub fn drain(&self) -> Vec<PlayerEvent> {
        let mut session = self.session.borrow_mut();
        let Some(session) = session.as_mut() else {
            return Vec::new();
        };
        let mut events = session.host.poll_events();
        if let Err(error) = session.host.retry_if_capacity() {
            events.extend([PlayerEvent::Error(error), PlayerEvent::Shutdown]);
        }
        for _ in 0..3 {
            if session
                .host
                .copy_ready(&session.context.queue, &mut session.current)
                .is_none()
            {
                break;
            }
            session.statistics.copies += 1;
        }
        events
    }

    pub fn execute(&self, command: Command) -> Result<(), String> {
        let mut session = self.session.borrow_mut();
        let session = session.as_mut().ok_or("player has no graphics session")?;
        session.host.execute(command)
    }

    pub fn statistics(&self) -> Statistics {
        self.session
            .borrow()
            .as_ref()
            .map_or(Statistics::default(), |s| s.statistics)
    }

    /// A normal iced shader widget; clipping, layout, overlays and screenshots
    /// follow the renderer's existing path. Pixels retain mpv's SDR code values.
    pub fn view<Message>(&self) -> impl iced::Widget<Message, iced::Theme, iced_wgpu::Renderer> {
        let texture = self.session.borrow().as_ref().map(|s| s.current.clone());
        iced_widget::shader(video::Frame::new(texture))
            .width(iced::Fill)
            .height(iced::Fill)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        eprintln!(
            "SDR presentation requests={} GPU copies={} last_present_elapsed={:?}s source_pts=unavailable",
            self.statistics.presentations,
            self.statistics.copies,
            self.statistics.last_presented_at
        );
    }
}
