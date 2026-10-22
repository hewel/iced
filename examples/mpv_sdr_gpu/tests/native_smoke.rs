//! Opt-in real desktop acceptance. Requires a >= 10 second moving video and
//! a compositor that honors fullscreen requests. Ordinary cargo test is headless.
use iced::{Task, Widget, window};
use mpv_sdr_gpu::{Command, Player, PlayerEvent, Settings, compositor::Compositor};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

#[derive(clap::Parser)]
struct Options {
    #[command(flatten)]
    player: Settings,
    #[arg(long)]
    report: PathBuf,
}

#[derive(Debug, Clone)]
enum Message {
    Wake,
    Opened(window::Id),
    Closed(window::Id),
    Shot(window::Screenshot),
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Warmup,
    Shot,
    Pause,
    Resize,
    Seek,
    Closing,
    Reopen,
    Resume,
    Stop,
    Reload,
    Done,
}

struct Smoke {
    player: Player,
    window: Option<window::Id>,
    phase: Phase,
    shot: usize,
    mark: u64,
    time: f64,
    paused: bool,
    idle: bool,
    source: String,
    report: PathBuf,
    outcome: Rc<RefCell<Result<(), String>>>,
    overlay: iced::widget::image::Handle,
}

impl Smoke {
    fn fail(&mut self, error: impl ToString) -> Task<Message> {
        *self.outcome.borrow_mut() = Err(error.to_string());
        iced::exit()
    }
    fn command(&mut self, command: Command) -> Task<Message> {
        match self.player.execute(command) {
            Ok(()) => Task::none(),
            Err(error) => self.fail(error),
        }
    }
    fn screenshot(&mut self) -> Task<Message> {
        self.phase = Phase::Shot;
        window::screenshot(self.window.expect("open window")).map(Message::Shot)
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Wake => {
                for event in self.player.drain() {
                    match event {
                        PlayerEvent::Error(error) => return self.fail(error),
                        PlayerEvent::Shutdown => return self.fail("unexpected mpv shutdown"),
                        PlayerEvent::Pause(Some(value)) => self.paused = value,
                        PlayerEvent::TimePos(Some(value)) => self.time = value,
                        PlayerEvent::Idle(Some(value)) => self.idle = value,
                        _ => {}
                    }
                }
                let copies = self.player.statistics().copies;
                match self.phase {
                    Phase::Warmup if copies >= 5 && self.window.is_some() => {
                        return self.screenshot();
                    }
                    Phase::Pause if self.paused => {
                        self.phase = Phase::Resize;
                        self.mark = copies;
                        return window::set_mode(self.window.unwrap(), window::Mode::Fullscreen);
                    }
                    Phase::Resize if copies > self.mark => return self.screenshot(),
                    Phase::Seek if self.time >= 4.9 && copies > self.mark => {
                        return self.screenshot();
                    }
                    Phase::Resume if !self.paused && self.time > 5.5 && copies > self.mark => {
                        self.phase = Phase::Stop;
                        return self.command(Command::Stop);
                    }
                    Phase::Stop if self.idle => {
                        self.phase = Phase::Reload;
                        self.mark = copies;
                        return self.command(Command::LoadFile(self.source.clone()));
                    }
                    Phase::Reload if !self.idle && copies > self.mark + 3 => {
                        return self.screenshot();
                    }
                    _ => {}
                }
            }
            Message::Opened(id) => {
                self.window = Some(id);
                if self.phase == Phase::Reopen {
                    if self.player.statistics().copies < self.mark {
                        return self.fail("native session reset on reopen");
                    }
                    return self.screenshot();
                }
            }
            Message::Closed(id) => {
                if self.window != Some(id) {
                    return self.fail("closed an unexpected window");
                }
                self.window = None;
                if self.phase == Phase::Closing {
                    self.phase = Phase::Reopen;
                    let (_, open) = window::open(window::Settings::default());
                    return open.map(Message::Opened);
                }
                if self.phase == Phase::Done {
                    *self.outcome.borrow_mut() = Ok(());
                    return iced::exit();
                }
                return self.fail("unexpected window close");
            }
            Message::Shot(screenshot) => {
                let pixels = screenshot.rgba.as_chunks::<4>().0.iter();
                let colored = pixels
                    .clone()
                    .filter(|p| p[0].abs_diff(p[1]) > 25 || p[1].abs_diff(p[2]) > 25)
                    .count();
                let overlay = pixels
                    .filter(|p| p[0] < 10 && p[1].abs_diff(231) < 10 && p[2].abs_diff(77) < 10)
                    .count();
                if colored < 1000 || overlay < 64 {
                    return self.fail(format!(
                        "missing video/image pixels: colored={colored} overlay={overlay}"
                    ));
                }
                let metadata = format!(
                    "{}x{} colored={} overlay={} copies={}\n",
                    screenshot.size.width,
                    screenshot.size.height,
                    colored,
                    overlay,
                    self.player.statistics().copies
                );
                if let Err(error) = std::fs::write(
                    self.report.join(format!("{}.rgba", self.shot)),
                    &screenshot.rgba,
                )
                .and_then(|_| {
                    std::fs::write(self.report.join(format!("{}.txt", self.shot)), metadata)
                }) {
                    return self.fail(error);
                }
                eprintln!("native smoke screenshot {} passed", self.shot);
                self.shot += 1;
                match self.shot {
                    1 => {
                        self.phase = Phase::Pause;
                        return self.command(Command::TogglePause);
                    }
                    2 => {
                        self.phase = Phase::Seek;
                        self.mark = self.player.statistics().copies;
                        return self.command(Command::SeekAbsolute(5.0));
                    }
                    3 => {
                        self.phase = Phase::Closing;
                        self.mark = self.player.statistics().copies;
                        return window::close(self.window.unwrap());
                    }
                    4 => {
                        self.phase = Phase::Resume;
                        self.mark = self.player.statistics().copies;
                        return self.command(Command::TogglePause);
                    }
                    5 => {
                        self.phase = Phase::Done;
                        return window::close(self.window.unwrap());
                    }
                    _ => return self.fail("unexpected screenshot"),
                }
            }
            Message::Timeout => {
                return self.fail(format!("native smoke timed out in {:?}", self.phase));
            }
        }
        Task::none()
    }
    fn view(&self, _: window::Id) -> impl Widget<Message, iced::Theme, iced_wgpu::Renderer> {
        iced::widget::stack![
            self.player.view(),
            iced::widget::image(self.overlay.clone())
                .width(64)
                .height(64)
        ]
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    let options = Options::parse();
    if options.player.pause {
        return Err("native smoke starts with playback; omit --pause".into());
    }
    std::fs::create_dir_all(&options.report)?;
    let source = options.player.source.clone();
    let (player, receiver) = Player::new(options.player)?;
    let receiver = RefCell::new(Some(receiver));
    let outcome = Rc::new(RefCell::new(Err(
        "runtime exited without completing acceptance".into(),
    )));
    let boot_outcome = outcome.clone();
    let boot_player = player.clone();
    iced::daemon(
        move || {
            let (_, open) = window::open(window::Settings::default());
            let (sender, timeout) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(30));
                let _ = sender.send(());
            });
            (
                Smoke {
                    player: boot_player.clone(),
                    window: None,
                    phase: Phase::Warmup,
                    shot: 0,
                    mark: 0,
                    time: 0.0,
                    paused: false,
                    idle: false,
                    source: source.clone(),
                    report: options.report.clone(),
                    outcome: boot_outcome.clone(),
                    overlay: iced::widget::image::Handle::from_rgba(
                        32,
                        32,
                        [1, 231, 77, 255].repeat(32 * 32),
                    ),
                },
                Task::batch([
                    open.map(Message::Opened),
                    Task::run(receiver.borrow_mut().take().unwrap(), |_| Message::Wake),
                    Task::perform(timeout, |_| Message::Timeout),
                ]),
            )
        },
        Smoke::update,
        Smoke::view,
    )
    .title("iced mpv native acceptance")
    .subscription(|_: &Smoke| window::close_events().map(Message::Closed))
    .run_with_compositor(move |_, display, window, shell| {
        Compositor::create(player.clone(), display, window, shell)
    })?;
    outcome.borrow().clone()?;
    eprintln!(
        "PASS: playback, image upload, screenshots, paused resize/seek, zero-window reopen, stop/reload, shutdown"
    );
    Ok(())
}
