mod controls;

use controls::Controls;
use iced::{Subscription, Task, Theme, Widget, window};
use mpv_sdr_gpu::{Player, PlayerEvent, Settings, compositor::Compositor};
use std::time::Duration;

#[derive(clap::Parser)]
#[command(about = "Linux Vulkan 10-bit SDR libmpv host with native iced controls")]
struct Options {
    #[command(flatten)]
    player: Settings,
    /// Exit after this many wall-clock seconds, including paused playback.
    #[arg(long)]
    duration: Option<f64>,
    /// Hide controls for pixel comparison without changing the video pipeline.
    #[arg(long)]
    no_overlay: bool,
}

struct App {
    player: Player,
    controls: Controls,
    no_overlay: bool,
    failure: std::rc::Rc<std::cell::RefCell<Option<String>>>,
}

#[derive(Debug, Clone)]
enum Message {
    Wake,
    Controls(controls::Message),
    Opened,
    Exit,
}

impl App {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Wake => {
                let events = self.player.drain();
                if events
                    .iter()
                    .any(|event| matches!(event, PlayerEvent::Shutdown))
                {
                    *self.failure.borrow_mut() = events.iter().find_map(|event| {
                        if let PlayerEvent::Error(error) = event {
                            Some(error.clone())
                        } else {
                            None
                        }
                    });
                    return iced::exit();
                }
                for event in events {
                    if let PlayerEvent::Error(error) = &event {
                        eprintln!("mpv: {error}");
                    }
                    self.controls.observe(event);
                }
                let statistics = self.player.statistics();
                self.controls.set_statistics(
                    statistics.presentations,
                    statistics.copies,
                    statistics.last_presented_at,
                );
            }
            Message::Controls(message) => {
                if let Some(command) = self.controls.update(message)
                    && let Err(error) = self.player.execute(command)
                {
                    self.controls.report_error(error);
                }
            }
            Message::Exit => return iced::exit(),
            Message::Opened => {}
        }
        Task::none()
    }

    fn view(&self, _window: window::Id) -> impl Widget<Message, Theme, iced_wgpu::Renderer> {
        let video = iced_widget::stack![self.player.view()];
        if self.no_overlay {
            video
        } else {
            video.push(self.controls.view().map(Message::Controls).boxed())
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    let options = Options::parse();
    options.player.validate()?;
    let duration = options
        .duration
        .map(Duration::try_from_secs_f64)
        .transpose()?;
    if duration.is_some_and(|duration| duration.is_zero()) {
        return Err("duration must be positive".into());
    }
    tracing_subscriber::fmt::init();
    let size = iced::Size::new(options.player.width as f32, options.player.height as f32);
    let (player, notifications) = Player::new(options.player)?;
    // Boot owns the stream once. The compositor factory may run again when a
    // daemon reopens a window; it reuses this same native session and queue gate.
    let boot_player = player.clone();
    let failure = std::rc::Rc::new(std::cell::RefCell::new(None));
    let boot_failure = failure.clone();
    let notifications = std::cell::RefCell::new(Some(notifications));
    iced::daemon(
        move || {
            let (_, open) = window::open(window::Settings {
                size,
                ..window::Settings::default()
            });
            let mut tasks = vec![
                open.map(|_| Message::Opened),
                Task::run(
                    notifications.borrow_mut().take().expect("boot once"),
                    |_| Message::Wake,
                ),
            ];
            if let Some(duration) = duration {
                let (sender, receiver) = futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    std::thread::sleep(duration);
                    let _ = sender.send(());
                });
                tasks.push(Task::perform(receiver, |_| Message::Exit));
            }
            (
                App {
                    player: boot_player.clone(),
                    controls: Controls::new(),
                    no_overlay: options.no_overlay,
                    failure: boot_failure.clone(),
                },
                Task::batch(tasks),
            )
        },
        App::update,
        App::view,
    )
    .title("mpv → iced · 10-bit SDR")
    .theme(Theme::Dark)
    .subscription(|_: &App| -> Subscription<Message> {
        window::close_events().map(|_| Message::Exit)
    })
    .run_with_compositor(move |_, display, window, shell| {
        Compositor::create(player.clone(), display, window, shell)
    })?;
    if let Some(error) = failure.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}
