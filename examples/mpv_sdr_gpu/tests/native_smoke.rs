//! Opt-in real desktop acceptance. Requires a >= 10 second moving video and
//! a compositor that honors fullscreen requests. Ordinary cargo test is headless.
use iced::{Task, Widget, window};
use iced_mpv_preview::{Frame, Preview};
use mpv_sdr_gpu::{Command, Player, PlayerEvent, Settings, compositor::Compositor};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

#[derive(clap::Parser)]
struct Options {
    #[command(flatten)]
    player: Settings,
    #[arg(long)]
    report: PathBuf,
    /// Exercise seek completion without requiring a fullscreen resize.
    #[arg(long)]
    seek_only: bool,
    /// Also decode thumbnails beside playing and paused main playback.
    #[arg(long)]
    preview: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Wake,
    PreviewWake,
    Frame,
    Opened(window::Id),
    Closed(window::Id),
    Shot(window::Screenshot),
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Warmup,
    PreviewPlaying,
    PreviewPaused,
    AwaitDraw,
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
    draw_mark: u64,
    time: f64,
    paused: bool,
    idle: bool,
    seek_finished: bool,
    seek_only: bool,
    source: String,
    report: PathBuf,
    outcome: Rc<RefCell<Result<(), String>>>,
    overlay: iced::widget::image::Handle,
    preview: Option<Rc<Preview>>,
    preview_frame: Option<Frame>,
    preview_count: usize,
    preview_mark: f64,
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
        // A screenshot captures the renderer's last draw, not a freshly rebuilt
        // view. Wait for presentation so a locked/occluded window cannot pass
        // with stale overlay-only pixels.
        self.phase = Phase::AwaitDraw;
        self.draw_mark = self.player.statistics().presentations;
        Task::none()
    }
    fn begin_previews(&mut self, phase: Phase) -> Task<Message> {
        self.phase = phase;
        self.preview_count = 0;
        self.preview_mark = self.time;
        self.preview.as_ref().unwrap().request(8.5);
        Task::none()
    }
    fn check_preview_playback(&mut self) -> Task<Message> {
        if self.phase == Phase::PreviewPlaying {
            if self.paused {
                return self.fail("preview paused the main player");
            }
            if self.preview_count == 4 && self.time > self.preview_mark {
                return self.screenshot();
            }
        } else if self.phase == Phase::PreviewPaused {
            if !self.paused || self.time != self.preview_mark {
                return self.fail("preview changed paused main playback");
            }
            if self.preview_count == 4 {
                return self.screenshot();
            }
        }
        Task::none()
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Wake => {
                for event in self.player.drain() {
                    match event {
                        PlayerEvent::Seeking(Some(true)) | PlayerEvent::PlaybackRestart
                            if matches!(
                                self.phase,
                                Phase::PreviewPlaying | Phase::PreviewPaused
                            ) =>
                        {
                            return self.fail("thumbnail seek reached the main player");
                        }
                        PlayerEvent::Error(error) => return self.fail(error),
                        PlayerEvent::Shutdown => return self.fail("unexpected mpv shutdown"),
                        PlayerEvent::Pause(Some(value)) => self.paused = value,
                        PlayerEvent::TimePos(Some(value)) => self.time = value,
                        PlayerEvent::Idle(Some(value)) => self.idle = value,
                        PlayerEvent::SeekFinished { request, result } => {
                            let position = match result {
                                Ok(position) => position,
                                Err(error) => return self.fail(error),
                            };
                            eprintln!(
                                "seek request={request} completed at {position:.6}s paused={}",
                                self.paused
                            );
                            if !self.paused || position < 5.25 {
                                return self
                                    .fail(format!("invalid paused seek result: {position}"));
                            }
                            if request == 1 {
                                // The second seek may produce no time-pos notification.
                                return self.command(Command::SeekAbsolute {
                                    request: 2,
                                    position,
                                });
                            }
                            self.time = position;
                            self.seek_finished = true;
                        }
                        _ => {}
                    }
                }
                let copies = self.player.statistics().copies;
                match self.phase {
                    Phase::Warmup
                        if copies >= if self.seek_only { 1 } else { 5 }
                            && self.window.is_some() =>
                    {
                        if self.preview.is_some() {
                            return self.begin_previews(Phase::PreviewPlaying);
                        }
                        return self.screenshot();
                    }
                    Phase::Pause if self.paused => {
                        if self.seek_only {
                            self.phase = Phase::Seek;
                            self.mark = copies;
                            return self.command(Command::SeekAbsolute {
                                request: 1,
                                position: 5.25,
                            });
                        }
                        self.phase = Phase::Resize;
                        self.mark = copies;
                        return window::set_mode(self.window.unwrap(), window::Mode::Fullscreen);
                    }
                    Phase::Resize if copies > self.mark => return self.screenshot(),
                    Phase::Seek if self.seek_finished && copies > self.mark => {
                        if self.preview.is_some() {
                            return self.begin_previews(Phase::PreviewPaused);
                        }
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
                    Phase::PreviewPlaying | Phase::PreviewPaused => {
                        return self.check_preview_playback();
                    }
                    _ => {}
                }
            }
            Message::PreviewWake => {
                if let Some(outcome) = self.preview.as_ref().and_then(|preview| preview.take()) {
                    let frame = match outcome.result {
                        Ok(frame) => frame,
                        Err(error) => return self.fail(error),
                    };
                    eprintln!(
                        "preview {:?}: requested={:.3} actual={:.6} main={:.6} paused={}",
                        self.phase, frame.requested, frame.actual, self.time, self.paused
                    );
                    self.preview_frame = Some(frame);
                    self.preview_count += 1;
                    if let Some(target) = [8.5, 0.5, 7.0, 3.3].get(self.preview_count) {
                        self.preview.as_ref().unwrap().request(*target);
                    }
                }
                return self.check_preview_playback();
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
            Message::Frame => {
                if self.phase == Phase::AwaitDraw
                    && self.player.statistics().presentations > self.draw_mark
                {
                    self.phase = Phase::Shot;
                    return window::screenshot(self.window.expect("open window"))
                        .map(Message::Shot);
                }
            }
            Message::Shot(screenshot) => {
                let pixels = screenshot.rgba.as_chunks::<4>().0.iter();
                let colored = pixels
                    .clone()
                    // The top-left overlays must not satisfy the video check.
                    .skip(screenshot.rgba.len() / 8)
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
                    2 if self.seek_only => {
                        self.phase = Phase::Done;
                        return window::close(self.window.unwrap());
                    }
                    2 => {
                        self.phase = Phase::Seek;
                        self.mark = self.player.statistics().copies;
                        return self.command(Command::SeekAbsolute {
                            request: 1,
                            position: 5.25,
                        });
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
        let mut overlay = iced::widget::row![
            iced::widget::image(self.overlay.clone())
                .width(64)
                .height(64)
        ]
        .spacing(16);
        if let Some(frame) = &self.preview_frame {
            overlay = overlay.push(
                iced::widget::image(frame.image.clone())
                    .width(320)
                    .height(180)
                    .boxed(),
            );
        }
        iced::widget::stack![self.player.view(), overlay]
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
    let (preview, preview_notifications) = if options.preview {
        let (preview, receiver) = Preview::new(options.player.libmpv.clone())?;
        preview.set_media(Some(&source));
        (Some(Rc::new(preview)), Some(receiver))
    } else {
        (None, None)
    };
    let boot_preview = preview.clone();
    let preview_notifications = RefCell::new(preview_notifications);
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
                    draw_mark: 0,
                    time: 0.0,
                    paused: false,
                    idle: false,
                    seek_finished: false,
                    source: source.clone(),
                    report: options.report.clone(),
                    seek_only: options.seek_only,
                    outcome: boot_outcome.clone(),
                    preview: boot_preview.clone(),
                    preview_frame: None,
                    preview_count: 0,
                    preview_mark: 0.0,
                    overlay: iced::widget::image::Handle::from_rgba(
                        32,
                        32,
                        [1, 231, 77, 255].repeat(32 * 32),
                    ),
                },
                Task::batch([
                    open.map(Message::Opened),
                    Task::run(receiver.borrow_mut().take().unwrap(), |_| Message::Wake),
                    preview_notifications
                        .borrow_mut()
                        .take()
                        .map_or_else(Task::none, |receiver| {
                            Task::run(receiver, |_| Message::PreviewWake)
                        }),
                    Task::perform(timeout, |_| Message::Timeout),
                ]),
            )
        },
        Smoke::update,
        Smoke::view,
    )
    .title("iced mpv native acceptance")
    .subscription(|_: &Smoke| {
        iced::Subscription::batch([
            window::close_events().map(Message::Closed),
            window::frames().map(|_| Message::Frame),
        ])
    })
    .run_with_compositor(move |_, display, window, shell| {
        Compositor::create(player.clone(), display, window, shell)
    })?;
    drop(preview);
    outcome.borrow().clone()?;
    if options.preview {
        eprintln!("PASS: independent thumbnails preserve playing/paused main playback");
    }
    if options.seek_only {
        eprintln!(
            "PASS: paused fractional and same-position seek completion, screenshots, shutdown"
        );
    } else {
        eprintln!(
            "PASS: playback, image upload, screenshots, paused resize/seek, zero-window reopen, stop/reload, shutdown"
        );
    }
    Ok(())
}
