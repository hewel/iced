use iced_wgpu::wgpu;
use std::{sync::mpsc, thread};

/// Sleeps until a submission exists, then waits for precisely that submission.
/// Neither the queue gate nor a pool lock is held during polling or joining.
pub(crate) struct Completion {
    sender: Option<mpsc::Sender<(wgpu::SubmissionIndex, usize)>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Completion {
    pub fn new(
        device: wgpu::Device,
        completed: impl Fn(usize, Result<(), String>) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("mpv-gpu-completion".into())
            .spawn(move || {
                while let Ok((submission, token)) = receiver.recv() {
                    let result = device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: None,
                        })
                        .map(|_| ())
                        .map_err(|error| error.to_string());
                    completed(token, result);
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    pub fn track(&self, submission: wgpu::SubmissionIndex, token: usize) {
        self.sender
            .as_ref()
            .expect("completion worker running")
            .send((submission, token))
            .expect("completion worker alive");
    }

    pub fn shutdown(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.shutdown();
    }
}
