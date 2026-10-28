//! Independent CPU thumbnails with explicit SDR/HDR to sRGB conversion.
mod color;
mod software;

use iced_winit::core::image;
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 180;
pub const CACHE_FRAMES: usize = 8;
const SEEK_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct Frame {
    pub requested: f64,
    pub actual: f64,
    pub color: &'static str,
    pub image: image::Handle,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub requested: f64,
    pub result: Result<Frame, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Request {
    media: u64,
    revision: u64,
    tick: u64,
}
impl Request {
    fn seconds(self) -> f64 {
        self.tick as f64 / 10.0
    }
}

#[derive(Default)]
struct Mailbox {
    media: u64,
    revision: u64,
    source: Option<Arc<str>>,
    target: Option<Request>,
    output: Option<(Request, Outcome)>,
    shutdown: bool,
}
impl Mailbox {
    fn invalidate(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("preview revision exhausted");
        self.target = None;
        self.output = None;
    }
    fn publish(&mut self, request: Request, result: Result<Frame, String>) -> bool {
        if self.shutdown || self.target != Some(request) {
            return false;
        }
        self.output = Some((
            request,
            Outcome {
                requested: request.seconds(),
                result,
            },
        ));
        true
    }
    fn take(&mut self) -> Option<Outcome> {
        self.output
            .take()
            .and_then(|(request, outcome)| (self.target == Some(request)).then_some(outcome))
    }
}

struct Shared {
    mailbox: Mutex<Mailbox>,
    worker: OnceLock<thread::Thread>,
    ui: Mutex<futures::channel::mpsc::Sender<()>>,
    #[cfg(test)]
    ack_gate: Mutex<Option<tests::AckGate>>,
    #[cfg(test)]
    shutdown_signal: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}
impl Shared {
    fn wake(&self) {
        if let Some(worker) = self.worker.get() {
            worker.unpark();
        }
    }
    fn publish(&self, request: Request, result: Result<Frame, String>) {
        if self.mailbox.lock().publish(request, result) {
            let _ = self.ui.lock().try_send(());
        }
    }
}

/// Keep the owner alive until the window runtime exits. Drop joins the worker;
/// request, cancel, set_media and take never call native APIs or wait for decoding.
pub struct Preview {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Preview {
    pub fn new(library: String) -> std::io::Result<(Self, futures::channel::mpsc::Receiver<()>)> {
        let (sender, receiver) = futures::channel::mpsc::channel(1);
        let shared = Arc::new(Shared {
            mailbox: Mutex::new(Mailbox::default()),
            worker: OnceLock::new(),
            ui: Mutex::new(sender),
            #[cfg(test)]
            ack_gate: Mutex::new(None),
            #[cfg(test)]
            shutdown_signal: Mutex::new(None),
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("mpv-thumbnail".into())
            .spawn(move || {
                let _ = state.worker.set(thread::current());
                run(state, library);
            })?;
        Ok((
            Self {
                shared,
                worker: Some(worker),
            },
            receiver,
        ))
    }

    /// Every call starts a new media generation, including reloading the same path.
    pub fn set_media(&self, source: Option<String>) {
        let mut mailbox = self.shared.mailbox.lock();
        mailbox.invalidate();
        mailbox.media = mailbox
            .media
            .checked_add(1)
            .expect("preview media generation exhausted");
        mailbox.source = source.map(Arc::from);
        drop(mailbox);
        self.shared.wake();
    }

    /// Coalesces to one latest target at 100 ms resolution. Returns the requested
    /// preview time, not the decoded frame's timestamp. No source means disabled.
    pub fn request(&self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        let mut mailbox = self.shared.mailbox.lock();
        if mailbox.source.is_none() || mailbox.shutdown {
            return None;
        }
        let tick = (seconds * 10.0).round() as u64;
        if mailbox.target.is_none_or(|request| request.tick != tick) {
            mailbox.invalidate();
            mailbox.target = Some(Request {
                media: mailbox.media,
                revision: mailbox.revision,
                tick,
            });
        }
        drop(mailbox);
        self.shared.wake();
        Some(tick as f64 / 10.0)
    }

    pub fn cancel(&self) {
        let mut mailbox = self.shared.mailbox.lock();
        if mailbox.target.is_some() {
            mailbox.invalidate();
        }
        drop(mailbox);
        self.shared.wake();
    }

    /// Rechecks request identity at delivery, not only at worker publication.
    pub fn take(&self) -> Option<Outcome> {
        self.shared.mailbox.lock().take()
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        {
            let mut mailbox = self.shared.mailbox.lock();
            mailbox.shutdown = true;
            mailbox.invalidate();
        }
        #[cfg(test)]
        if let Some(signal) = self.shared.shutdown_signal.lock().take() {
            let _ = signal.send(());
        }
        self.shared.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
struct Cache(VecDeque<(u64, Frame)>);
impl Cache {
    fn get(&mut self, tick: u64) -> Option<Frame> {
        let index = self.0.iter().position(|(key, _)| *key == tick)?;
        let entry = self.0.remove(index)?;
        let frame = entry.1.clone();
        self.0.push_back(entry);
        Some(frame)
    }
    fn insert(&mut self, tick: u64, frame: Frame) {
        self.0.retain(|(key, _)| *key != tick);
        if self.0.len() == CACHE_FRAMES {
            self.0.pop_front();
        }
        self.0.push_back((tick, frame));
    }
}

fn run(shared: Arc<Shared>, library: String) {
    let mut media = 0;
    let mut decoder: Option<software::Decoder> = None;
    let mut cache = Cache::default();
    let mut failure: Option<String> = None;
    let mut delivered = None;
    let mut next_seek = Instant::now();
    loop {
        let (shutdown, generation, source, target) = {
            let mailbox = shared.mailbox.lock();
            (
                mailbox.shutdown,
                mailbox.media,
                mailbox.source.clone(),
                mailbox.target,
            )
        };
        if shutdown {
            break;
        }
        if media != generation {
            decoder = None;
            media = generation;
            cache.0.clear();
            failure = None;
            delivered = None;
        }
        if let Some(decoder) = &mut decoder {
            match decoder.drain() {
                Ok(Some((request, frame))) => {
                    cache.insert(request.tick, frame.clone());
                    shared.publish(request, Ok(frame));
                    delivered = Some(request);
                }
                Err(error) => failure = Some(error),
                Ok(None) => {}
            }
        }
        if failure.is_none()
            && let Some(plan) = decoder
                .as_mut()
                .and_then(|native| native.reconfigure.take())
        {
            // Never keep two cores alive. No probe pixels enter the cache/UI.
            decoder = None;
            if let Some(source) = &source {
                match software::Decoder::new(&library, source, shared.clone(), Some(plan)) {
                    Ok(native) => decoder = Some(native),
                    Err(error) => failure = Some(error),
                }
            }
            continue;
        }
        let mut deadline = None;
        if let Some(request) = target.filter(|request| Some(*request) != delivered) {
            if let Some(error) = &failure {
                shared.publish(request, Err(error.clone()));
                delivered = Some(request);
            } else if let Some(frame) = cache.get(request.tick) {
                shared.publish(request, Ok(frame));
                delivered = Some(request);
            } else if decoder.is_none() {
                if let Some(source) = source {
                    match software::Decoder::new(&library, &source, shared.clone(), None) {
                        Ok(native) => decoder = Some(native),
                        Err(error) => failure = Some(error),
                    }
                    // Initialization can produce events before the first park.
                    continue;
                }
            } else if let Some(native) = &mut decoder
                && native.idle()
            {
                if Instant::now() >= next_seek {
                    if let Err(error) = native.seek(request) {
                        failure = Some(error);
                        continue;
                    }
                    next_seek = Instant::now() + SEEK_INTERVAL;
                } else {
                    deadline = Some(next_seek);
                }
            }
        }
        if failure.is_some() {
            // Publish the failure before native teardown, which is a blocking
            // library boundary and is not covered by our operation deadline.
            decoder = None;
        }
        if let Some(native_deadline) = decoder.as_ref().and_then(software::Decoder::deadline) {
            deadline = Some(deadline.map_or(native_deadline, |at| at.min(native_deadline)));
        }
        // Wake callbacks only unpark. The token survives a wake between checking
        // state and parking; idle playback never needs a periodic poll timer.
        if let Some(deadline) = deadline {
            thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
        } else {
            thread::park();
        }
    }
    // Native teardown occurs here on the worker, never under the mailbox lock.
}

#[cfg(test)]
mod tests {
    use super::*;
    mod native;

    pub(super) enum AckAction {
        Continue,
        SuppressCompletion,
    }
    pub(super) struct AckGate {
        pub(super) reached: std::sync::mpsc::Sender<Request>,
        pub(super) proceed: std::sync::mpsc::Receiver<AckAction>,
    }

    #[test]
    fn late_result_cannot_replace_latest_or_survive_cancel_and_media_change() {
        let mut mailbox = Mailbox::default();
        let a = Request {
            media: 1,
            revision: 1,
            tick: 10,
        };
        let c = Request {
            media: 1,
            revision: 3,
            tick: 90,
        };
        mailbox.target = Some(c);
        assert!(!mailbox.publish(a, Err("old A".into())));
        assert!(mailbox.publish(c, Err("current C".into())));
        assert!(!mailbox.publish(a, Err("late A".into())));
        assert_eq!(mailbox.take().unwrap().result.unwrap_err(), "current C");
        assert!(mailbox.publish(c, Err("current C".into())));
        mailbox.invalidate();
        assert!(mailbox.take().is_none());
        assert!(!mailbox.publish(c, Err("late canceled C".into())));
        mailbox.target = Some(Request {
            media: 2,
            revision: 4,
            tick: 90,
        });
        assert!(!mailbox.publish(c, Err("previous file".into())));
        mailbox.shutdown = true;
        assert!(!mailbox.publish(mailbox.target.unwrap(), Err("after shutdown".into())));
    }

    #[test]
    fn cache_evicts_least_recently_used_without_changing_pixel_identity() {
        let mut cache = Cache::default();
        for tick in 0..CACHE_FRAMES as u64 {
            cache.insert(
                tick,
                Frame {
                    requested: tick as f64,
                    actual: tick as f64,
                    color: "SDR → sRGB",
                    image: image::Handle::from_rgba(1, 1, [tick as u8, 0, 0, 255].to_vec()),
                },
            );
        }
        let retained = cache.get(0).unwrap().image.id();
        cache.insert(
            99,
            Frame {
                requested: 99.0,
                actual: 99.0,
                color: "SDR → sRGB",
                image: image::Handle::from_rgba(1, 1, vec![99, 0, 0, 255]),
            },
        );
        assert!(cache.get(1).is_none());
        assert_eq!(cache.get(0).unwrap().image.id(), retained);
        assert_eq!(cache.0.len(), CACHE_FRAMES);
    }
}
