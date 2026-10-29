use iced_mpv_preview as preview;
use iced_wgpu::Renderer;
use iced_widget::timeline::{Action, Timeline};
use iced_widget::{
    Widget, bottom, button, checkbox, column, container, row, scrollable, slider, text,
};
use iced_winit::core::{Font, Pixels, Theme};
use mpv_sdr_gpu::{Command, PlayerEvent};

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
    pending_seek: Option<f64>,
    seek_request: u64,
    media_revision: u64,
    thumbnail_target: Option<f64>,
    thumbnail: Option<Result<preview::Frame, String>>,
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
    Timeline(Action),
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
            pending_seek: None,
            seek_request: 0,
            media_revision: 0,
            thumbnail_target: None,
            thumbnail: None,
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
            Message::Timeline(Action::Preview(position))
                if self.can_seek_absolute() && position.is_finite() =>
            {
                self.seek_preview = Some(position.clamp(0.0, self.duration.unwrap()));
            }
            Message::Timeline(Action::Commit(position))
                if self.can_seek_absolute() && position.is_finite() =>
            {
                self.seek_preview = None;
                let target = position.clamp(0.0, self.duration.unwrap());
                return Some(self.commit_seek(target));
            }
            Message::Timeline(Action::Cancel) => self.seek_preview = None,
            Message::SeekRelative(offset) if self.can_seek() && offset.is_finite() => {
                self.seek_preview = None;
                if self.can_seek_absolute() {
                    let target = (self.pending_seek.or(self.time_pos).unwrap() + offset)
                        .clamp(0.0, self.duration.unwrap());
                    return Some(self.commit_seek(target));
                }
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
                self.seek_preview = self.seek_preview.zip(self.duration).map(|(p, d)| p.min(d));
                self.pending_seek = self.pending_seek.zip(self.duration).map(|(p, d)| p.min(d));
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
                self.media_revision = self
                    .media_revision
                    .checked_add(1)
                    .expect("media revision exhausted");
                // Unchanged properties need not be re-emitted across files.
                // Keep observer values, but hide file-local data until loaded.
            }
            PlayerEvent::FileLoaded => self.file_state = FileState::Ready,
            PlayerEvent::PlaybackRestart | PlayerEvent::MediaPath(_) => {}
            PlayerEvent::SeekFinished { request, result } => {
                if self.pending_seek.is_some() && request == self.seek_request {
                    self.pending_seek = None;
                    match result {
                        Ok(position) => self.time_pos = Some(position),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            PlayerEvent::EndFile { error } => {
                self.file_state = FileState::Ended;
                self.seek_preview = None;
                if let Some(error) = error {
                    self.report_error(error);
                }
            }
            PlayerEvent::Error(error) => self.error = Some(error),
            PlayerEvent::Shutdown => {
                self.file_state = FileState::Shutdown;
                self.seek_preview = None;
            }
        }
        if !self.can_seek_absolute() {
            self.seek_preview = None;
            self.pending_seek = None;
            self.preview_target(None);
        }
    }

    pub fn preview_target(&mut self, target: Option<f64>) {
        if self.thumbnail_target != target {
            self.thumbnail_target = target;
            self.thumbnail = None;
        }
    }

    pub fn preview_outcome(&mut self, outcome: preview::Outcome) {
        if self.thumbnail_target == Some(outcome.requested) {
            self.thumbnail = Some(outcome.result);
        }
    }

    pub fn preview_enabled(&self) -> bool {
        self.can_seek_absolute()
    }

    fn commit_seek(&mut self, position: f64) -> Command {
        self.seek_request = self
            .seek_request
            .checked_add(1)
            .expect("seek request IDs exhausted");
        self.pending_seek = Some(position);
        Command::SeekAbsolute {
            request: self.seek_request,
            position,
        }
    }

    pub fn report_error(&mut self, error: String) {
        self.pending_seek = None;
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

        let preview_content = self.thumbnail_target.map(|target| {
            let surface = match &self.thumbnail {
                Some(Ok(frame)) => iced_widget::image(&frame.image)
                    .width(preview::WIDTH)
                    .height(preview::HEIGHT)
                    .boxed(),
                Some(Err(error)) => container(text(error).size(14))
                    .padding(12)
                    .width(preview::WIDTH)
                    .height(preview::HEIGHT)
                    .boxed(),
                None => container(text("Loading preview…").size(14))
                    .padding(12)
                    .width(preview::WIDTH)
                    .height(preview::HEIGHT)
                    .boxed(),
            };
            let caption = match &self.thumbnail {
                Some(Ok(frame)) => format!(
                    "Target {target:.1} s · frame {:.2} s · {}",
                    frame.actual, frame.color
                ),
                _ => format!("Preview target {target:.1} s"),
            };
            container(column![surface, text(caption).size(14)].spacing(4))
                .padding(8)
                .style(container::dark)
                .boxed()
        });
        if self.can_seek_absolute() {
            let duration = self.duration.unwrap_or(0.0);
            let mut timeline = Timeline::new(
                0.0..=duration,
                self.seek_preview
                    .or(self.pending_seek)
                    .or(self.time_pos)
                    .unwrap_or(0.0),
                Message::Timeline,
            )
            .generation(self.media_revision)
            .step(5.0)
            .shift_step(1.0);
            if let Some(preview) = preview_content {
                timeline = timeline.preview(preview);
            }
            content = content.push(timeline.boxed());
        } else {
            content = content.push(
                text(
                    "Timeline unavailable: requires active seekable media, position, and duration",
                )
                .size(14)
                .boxed(),
            );
        }
        content = content.push(
            text(if let Some(preview) = self.seek_preview {
                format!(
                    "Seek to {} · release inside to seek · Esc to cancel",
                    time_label(Some(preview))
                )
            } else if let Some(target) = self.pending_seek {
                format!("Seeking to {}", time_label(Some(target)))
            } else {
                "Drag to seek · arrows: 5 s · Shift+arrows: 1 s · Home / End".to_owned()
            })
            .size(14)
            .font(Font::MONOSPACE)
            .boxed(),
        );

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

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Controls {
        let mut controls = Controls::new();
        for event in [
            PlayerEvent::Duration(Some(120.0)),
            PlayerEvent::TimePos(Some(10.0)),
            PlayerEvent::Seekable(Some(true)),
            PlayerEvent::Idle(Some(false)),
            PlayerEvent::FileLoaded,
        ] {
            controls.observe(event);
        }
        controls
    }

    #[test]
    fn stale_restart_and_position_do_not_acknowledge_new_seek() {
        let mut controls = ready();
        controls.update(Message::Timeline(Action::Commit(30.0)));
        controls.update(Message::Timeline(Action::Commit(50.0)));
        controls.observe(PlayerEvent::PlaybackRestart);
        controls.observe(PlayerEvent::TimePos(Some(30.0)));
        assert_eq!(controls.pending_seek, Some(50.0));
        controls.observe(PlayerEvent::SeekFinished {
            request: 1,
            result: Ok(30.0),
        });
        assert_eq!(controls.pending_seek, Some(50.0));
        controls.observe(PlayerEvent::SeekFinished {
            request: 2,
            result: Ok(51.0),
        });
        assert_eq!(controls.pending_seek, None);
        assert_eq!(controls.time_pos, Some(51.0));
        controls.update(Message::Timeline(Action::Commit(51.0)));
        controls.observe(PlayerEvent::SeekFinished {
            request: 3,
            result: Ok(51.0),
        });
        assert_eq!(controls.pending_seek, None);
    }

    #[test]
    fn cancel_does_not_seek_and_file_change_invalidates_targets() {
        let mut controls = ready();
        assert!(
            controls
                .update(Message::Timeline(Action::Preview(60.0)))
                .is_none()
        );
        assert!(controls.update(Message::Timeline(Action::Cancel)).is_none());
        assert_eq!(controls.seek_preview, None);
        assert_eq!(controls.time_pos, Some(10.0));
        controls.update(Message::Timeline(Action::Commit(50.0)));
        controls.observe(PlayerEvent::StartFile);
        assert_eq!(controls.pending_seek, None);
        assert!(
            controls
                .update(Message::Timeline(Action::Commit(60.0)))
                .is_none()
        );
    }

    #[test]
    fn repeated_relative_actions_accumulate_on_latest_target() {
        let mut controls = ready();
        controls.update(Message::SeekRelative(10.0));
        assert!(matches!(
            controls.update(Message::SeekRelative(10.0)),
            Some(Command::SeekAbsolute { position: 30.0, .. })
        ));
        assert_eq!(controls.pending_seek, Some(30.0));
        assert!(
            controls
                .update(Message::Timeline(Action::Commit(f64::NAN)))
                .is_none()
        );
        assert_eq!(controls.pending_seek, Some(30.0));
    }
}
