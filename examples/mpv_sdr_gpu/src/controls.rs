use crate::interop::{Command, PlayerEvent};
use iced_wgpu::Renderer;
use iced_widget::{
    Widget, bottom, button, checkbox, column, container, row, scrollable, slider, text,
};
use iced_winit::core::{Font, Pixels, Theme};

pub struct Controls {
    show_details: bool,
    time_pos: Option<f64>,
    duration: Option<f64>,
    pause: Option<bool>,
    volume: Option<f64>,
    idle: Option<bool>,
    seeking: Option<bool>,
    buffering: Option<bool>,
    seekable: Option<bool>,
    file_state: FileState,
    seek_preview: Option<f64>,
    error: Option<String>,
    presentations: u64,
    copies: u64,
    last_presented_at: Option<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum FileState {
    Loading,
    Ready,
    Ended,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum Message {
    ShowDetails(bool),
    TogglePause,
    PreviewSeek(f64),
    CommitSeek,
    SeekRelative(f64),
    SetVolume(f64),
    DismissError,
}

impl Controls {
    pub fn new() -> Self {
        Self {
            show_details: false,
            time_pos: None,
            duration: None,
            pause: None,
            volume: None,
            idle: None,
            seeking: None,
            buffering: None,
            seekable: None,
            file_state: FileState::Loading,
            seek_preview: None,
            error: None,
            presentations: 0,
            copies: 0,
            last_presented_at: None,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Command> {
        match message {
            Message::ShowDetails(show) => self.show_details = show,
            Message::TogglePause if self.active() && self.pause.is_some() => {
                return Some(Command::TogglePause);
            }
            Message::PreviewSeek(position) if self.can_seek_absolute() && position.is_finite() => {
                self.seek_preview = self.duration.map(|duration| position.clamp(0.0, duration));
            }
            Message::CommitSeek => {
                let preview = self.seek_preview.take();
                if self.can_seek_absolute() {
                    return preview.zip(self.duration).map(|(position, duration)| {
                        Command::SeekAbsolute(position.clamp(0.0, duration))
                    });
                }
            }
            Message::SeekRelative(offset) if self.can_seek() && offset.is_finite() => {
                self.seek_preview = None;
                return Some(Command::SeekRelative(offset));
            }
            Message::SetVolume(volume)
                if self.file_state != FileState::Shutdown
                    && self.volume.is_some()
                    && volume.is_finite()
                    && volume >= 0.0 =>
            {
                return Some(Command::SetVolume(volume));
            }
            Message::DismissError => self.error = None,
            _ => {}
        }
        None
    }

    pub fn observe(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::TimePos(value) => {
                self.time_pos = value.filter(|value| value.is_finite() && *value >= 0.0);
            }
            PlayerEvent::Duration(value) => {
                self.duration = value.filter(|value| value.is_finite() && *value >= 0.0);
            }
            PlayerEvent::Pause(value) => self.pause = value,
            PlayerEvent::Volume(value) => {
                self.volume = value.filter(|value| value.is_finite() && *value >= 0.0);
            }
            PlayerEvent::Idle(value) => self.idle = value,
            PlayerEvent::Seeking(value) => self.seeking = value,
            PlayerEvent::Buffering(value) => self.buffering = value,
            PlayerEvent::Seekable(value) => self.seekable = value,
            PlayerEvent::StartFile => {
                self.file_state = FileState::Loading;
                self.seek_preview = None;
                // Unchanged properties need not be re-emitted across files.
                // Keep observer values, but hide file-local data until loaded.
            }
            PlayerEvent::FileLoaded => self.file_state = FileState::Ready,
            PlayerEvent::EndFile { error } => {
                self.file_state = FileState::Ended;
                self.seek_preview = None;
                if let Some(error) = error {
                    self.report_error(error);
                }
            }
            PlayerEvent::Error(error) => self.report_error(error),
            PlayerEvent::Shutdown => {
                self.file_state = FileState::Shutdown;
                self.seek_preview = None;
            }
        }
        if !self.can_seek_absolute() {
            self.seek_preview = None;
        }
    }

    pub fn report_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn set_statistics(
        &mut self,
        presentations: u64,
        copies: u64,
        last_presented_at: Option<f64>,
    ) {
        self.presentations = presentations;
        self.copies = copies;
        self.last_presented_at = last_presented_at;
    }

    fn active(&self) -> bool {
        self.file_state == FileState::Ready && self.idle == Some(false)
    }

    fn can_seek(&self) -> bool {
        self.active() && self.seekable == Some(true)
    }

    fn can_seek_absolute(&self) -> bool {
        self.can_seek()
            && self.time_pos.is_some()
            && self.duration.is_some_and(|duration| duration > 0.0)
    }

    pub fn view(&self) -> impl Widget<Message, Theme, Renderer> {
        let status = match self.file_state {
            FileState::Shutdown => "mpv shut down",
            FileState::Loading => "Loading",
            FileState::Ended => "Ended",
            FileState::Ready if self.idle == Some(true) => "Idle",
            FileState::Ready if self.seeking == Some(true) => "Seeking",
            FileState::Ready if self.buffering == Some(true) => "Buffering",
            FileState::Ready if self.pause == Some(true) => "Paused",
            FileState::Ready if self.pause == Some(false) && self.idle == Some(false) => "Playing",
            FileState::Ready => "Playback state unavailable",
        };
        let pause_label = match self.pause {
            Some(true) => "Play",
            Some(false) => "Pause",
            None => "Pause unavailable",
        };
        let mut content = column![
            row![
                button(pause_label).height(40).on_press_maybe(
                    (self.active() && self.pause.is_some()).then_some(Message::TogglePause)
                ),
                button("−10 s")
                    .height(40)
                    .on_press_maybe(self.can_seek().then_some(Message::SeekRelative(-10.0))),
                button("+10 s")
                    .height(40)
                    .on_press_maybe(self.can_seek().then_some(Message::SeekRelative(10.0))),
                text(status).size(16),
            ]
            .spacing(10)
            .wrap(),
            text(format!(
                "mpv playback clock: {} / duration: {}",
                time_label(
                    self.time_pos
                        .filter(|_| self.file_state == FileState::Ready)
                ),
                time_label(
                    self.duration
                        .filter(|_| self.file_state == FileState::Ready)
                )
            ))
            .size(14)
            .font(Font::MONOSPACE),
        ]
        .spacing(6);

        if self.can_seek_absolute() {
            let duration = self.duration.unwrap_or(0.0);
            content = content.push(
                slider(
                    0.0..=duration,
                    self.seek_preview.or(self.time_pos).unwrap_or(0.0),
                    Message::PreviewSeek,
                )
                .step(0.1)
                .height(40)
                .on_release(Message::CommitSeek)
                .boxed(),
            );
        } else {
            content = content.push(
                text(
                    "Timeline unavailable: requires active seekable media, position, and duration",
                )
                .size(14)
                .boxed(),
            );
        }
        if let Some(preview) = self.seek_preview {
            content = content.push(
                row![
                    text(format!(
                        "Seek preview: {} · release slider or choose Seek",
                        time_label(Some(preview))
                    ))
                    .size(14),
                    button("Seek").height(40).on_press(Message::CommitSeek),
                ]
                .spacing(10)
                .wrap()
                .boxed(),
            );
        }

        let volume_label = self.volume.map_or_else(
            || "unavailable".to_owned(),
            |volume| format!("{volume:.0}%"),
        );
        content = content.push(
            text(format!(
                "Volume: {volume_label} · observed pause: {}",
                flag_label(self.pause)
            ))
            .size(14)
            .boxed(),
        );
        if let Some(volume) = self
            .volume
            .filter(|_| self.file_state != FileState::Shutdown)
        {
            content = content.push(
                slider(0.0..=volume.max(100.0), volume, Message::SetVolume)
                    .step(1.0)
                    .height(40)
                    .boxed(),
            );
        }
        content = content.push(
            text(format!(
                "Idle: {} · seeking: {} · buffering: {} · seekable: {}",
                flag_label(self.idle),
                flag_label(self.seeking.filter(|_| self.file_state == FileState::Ready)),
                flag_label(
                    self.buffering
                        .filter(|_| self.file_state == FileState::Ready)
                ),
                flag_label(
                    self.seekable
                        .filter(|_| self.file_state == FileState::Ready)
                ),
            ))
            .size(14)
            .boxed(),
        );

        if let Some(error) = &self.error {
            content = content.push(
                container(
                    row![
                        text(format!("Playback error: {error}")).size(14),
                        button("Dismiss").height(40).on_press(Message::DismissError),
                    ]
                    .spacing(10)
                    .wrap(),
                )
                .padding(8)
                .style(container::bordered_box)
                .boxed(),
            );
        }

        let toggle = checkbox(self.show_details)
            .label("Show video target details")
            .on_toggle(Message::ShowDetails)
            .size(22)
            .spacing(10)
            .text_size(14)
            .line_height(Pixels(40.0));
        content = content.push(toggle.boxed());
        if self.show_details {
            let last_presented = self.last_presented_at.map_or_else(
                || "unavailable".to_owned(),
                |seconds| format!("{seconds:.3} s"),
            );
            content = content.push(
                column![
                    text("10-bit SDR target").size(18),
                    text("Input / output: RGB10_A2 UNORM").size(14),
                    text("BT.709 · gamma 2.2 · full range").size(14),
                    text("White: 203 cd/m² · black: 0.203 cd/m²").size(14),
                    text("Premultiplied alpha over black").size(14),
                    text("Nearest sampling · no color conversion or readback").size(14),
                    text(format!("Presentations: {} · GPU copies: {}", self.presentations, self.copies)).size(14),
                    text(format!("Last frame.present: {last_presented} (host elapsed)")).size(14),
                    text("Source PTS: unavailable · playback clock is not the displayed image timestamp").size(14),
                ]
                .spacing(4)
                .boxed(),
            );
        }

        bottom(scrollable(
            container(content).padding([8, 14]).style(container::dark),
        ))
        .padding(16)
    }
}

fn flag_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unavailable",
    }
}

fn time_label(value: Option<f64>) -> String {
    value.map_or_else(
        || "unavailable".to_owned(),
        |seconds| format!("{seconds:.1} s"),
    )
}
