//! Opt-in real libmpv tests. Run inside the process-level timeout in verify_preview.sh.
use super::*;
use futures::StreamExt;
use std::sync::mpsc;

type Events = futures::channel::mpsc::Receiver<()>;
fn setup() -> (Preview, Events, String) {
    let library = std::env::var("LIBMPV").expect("set LIBMPV to the real shared library");
    let source = std::env::var("MPV_PREVIEW_TEST_MEDIA")
        .expect("set MPV_PREVIEW_TEST_MEDIA to seekable moving video");
    let (preview, mut events) = Preview::new(library).unwrap();
    preview.set_media(Some(&source));
    // Exercise the configured color pipeline, not only the disposable probe,
    // when the later test holds a native seek at its acknowledgement.
    let _ = preview.request(0.5);
    assert_eq!(next(&preview, &mut events).result.unwrap().requested, 0.5);
    (preview, events, source)
}
fn gate(preview: &Preview) -> (mpsc::Receiver<Request>, mpsc::Sender<AckAction>) {
    let (reached, receiver) = mpsc::channel();
    let (sender, proceed) = mpsc::channel();
    *preview.shared.ack_gate.lock() = Some(AckGate { reached, proceed });
    (receiver, sender)
}
fn next(preview: &Preview, events: &mut Events) -> Outcome {
    futures::executor::block_on(async {
        loop {
            if let Some(outcome) = preview.take() {
                return outcome;
            }
            events.next().await.expect("worker notifications closed");
        }
    })
}

#[test]
#[ignore = "real libmpv; run tests/verify_preview.sh with a process timeout"]
fn native_inflight_cancel_and_media_replacement() {
    let (preview, mut events, source) = setup();
    let (ack, resume) = gate(&preview);
    let _ = preview.request(8.5);
    let old = ack.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(old.seconds(), 8.5);
    // The worker has consumed a real command reply but cannot consume restart
    // until this barrier is released. Cancellation is therefore truly in flight.
    preview.cancel();
    let _ = preview.request(3.3);
    assert!(preview.take().is_none());
    resume.send(AckAction::Continue).unwrap();
    assert_eq!(next(&preview, &mut events).result.unwrap().requested, 3.3);

    let (ack, resume) = gate(&preview);
    let _ = preview.request(7.0);
    let _ = ack.recv_timeout(Duration::from_secs(10)).unwrap();
    preview.set_media(Some(&source));
    let _ = preview.request(0.5);
    resume.send(AckAction::Continue).unwrap();
    assert_eq!(next(&preview, &mut events).result.unwrap().requested, 0.5);
    drop(preview);
    eprintln!("PASS acknowledged in-flight cancellation and media replacement; worker joined");
}

#[test]
#[ignore = "real libmpv; run tests/verify_preview.sh with a process timeout"]
fn native_missing_completion_fails_and_reload_recovers() {
    let (preview, mut events, source) = setup();
    let (ack, resume) = gate(&preview);
    let _ = preview.request(8.5);
    let _ = ack.recv_timeout(Duration::from_secs(10)).unwrap();
    // Fault injection removes only completion events after a real native ack.
    // Render updates continue; the production deadline path must release the job.
    resume.send(AckAction::SuppressCompletion).unwrap();
    let error = next(&preview, &mut events).result.unwrap_err();
    eprintln!("accepted seek without completion: {error}");
    let _ = preview.request(7.0);
    assert!(
        next(&preview, &mut events).result.is_err(),
        "failure must not silently retry"
    );
    preview.set_media(Some(&source));
    let _ = preview.request(0.5);
    assert_eq!(next(&preview, &mut events).result.unwrap().requested, 0.5);
    drop(preview);
    eprintln!("PASS deadline failure, explicit later failure, reload recovery and worker join");
}

#[test]
#[ignore = "real libmpv; run tests/verify_preview.sh with a process timeout"]
fn native_shutdown_with_acknowledged_request() {
    let (preview, _events, _) = setup();
    let (ack, resume) = gate(&preview);
    let (closing, closed) = mpsc::channel();
    *preview.shared.shutdown_signal.lock() = Some(closing);
    let _ = preview.request(8.5);
    let _ = ack.recv_timeout(Duration::from_secs(10)).unwrap();
    let owner = thread::spawn(move || drop(preview));
    closed.recv_timeout(Duration::from_secs(2)).unwrap();
    // Shutdown is committed while the native request is still held at its ack.
    resume.send(AckAction::Continue).unwrap();
    owner.join().unwrap();
    eprintln!("PASS shutdown during acknowledged request; context/core teardown and join returned");
}
