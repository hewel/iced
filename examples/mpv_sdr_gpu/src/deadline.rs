//! One bounded wake timer per host; never one sleeping thread per seek.
use parking_lot::{Condvar, Mutex};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub(crate) const NATIVE_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct State {
    at: Option<Instant>,
    stopping: bool,
}

pub(crate) struct WakeTimer {
    shared: Arc<(Mutex<State>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
}
impl WakeTimer {
    pub(crate) fn new(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let signal = shared.clone();
        let worker = thread::Builder::new()
            .name("mpv-seek-deadline".into())
            .spawn(move || {
                let (state, changed) = &*signal;
                let mut state = state.lock();
                loop {
                    if state.stopping {
                        break;
                    }
                    if let Some(at) = state.at {
                        if Instant::now() >= at {
                            state.at = None;
                            drop(state);
                            wake();
                            state = signal.0.lock();
                        } else {
                            changed.wait_until(&mut state, at);
                        }
                    } else {
                        changed.wait(&mut state);
                    }
                }
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub(crate) fn arm(&self, at: Option<Instant>) {
        let mut state = self.shared.0.lock();
        if state.at != at {
            state.at = at;
            self.shared.1.notify_one();
        }
    }
}
impl Drop for WakeTimer {
    fn drop(&mut self) {
        self.shared.0.lock().stopping = true;
        self.shared.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expires_without_native_events_and_shutdown_interrupts_future_deadline() {
        let (send, receive) = std::sync::mpsc::channel();
        let timer = WakeTimer::new(move || {
            let _ = send.send(());
        })
        .unwrap();
        timer.arm(Some(Instant::now()));
        receive.recv_timeout(Duration::from_secs(2)).unwrap();
        timer.arm(Some(Instant::now() + Duration::from_secs(3600)));
        drop(timer);
        assert_eq!(
            receive.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        );
    }
}
