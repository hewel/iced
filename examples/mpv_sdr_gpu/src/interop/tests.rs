use super::*;

#[test]
fn accepted_seek_without_restart_fails_active_and_latest_request_once() {
    let now = Instant::now();
    let mut pending = Some(PendingSeek {
        request: 11,
        native_id: 100,
        acknowledged: true,
        started: false,
        snapshot: false,
        position: 1000.0,
        checking_capability: false,
        submission_pending: false,
        deadline: now + NATIVE_OPERATION_TIMEOUT,
    });
    let mut queued = Some((13, 20.0));
    let mut failed = false;
    let mut events = Vec::new();
    expire_seek(&mut pending, &mut queued, &mut failed, now, &mut events);
    assert!(pending.is_some());
    assert!(events.is_empty());
    expire_seek(
        &mut pending,
        &mut queued,
        &mut failed,
        now + NATIVE_OPERATION_TIMEOUT,
        &mut events,
    );
    assert!(failed);
    assert!(
        pending.is_none(),
        "late native events must have no request to complete"
    );
    assert!(
        queued.is_none(),
        "a new submission could consume the old restart"
    );
    assert!(matches!(
        events.as_slice(),
        [
            PlayerEvent::SeekFinished {
                request: 11,
                result: Err(_)
            },
            PlayerEvent::SeekFinished {
                request: 13,
                result: Err(_)
            },
        ]
    ));
    events.clear();
    expire_seek(
        &mut pending,
        &mut queued,
        &mut failed,
        now + NATIVE_OPERATION_TIMEOUT,
        &mut events,
    );
    assert!(
        events.is_empty(),
        "a completed failure must not be emitted again"
    );
}

#[test]
fn queued_lifecycle_events_discard_a_ready_capability_before_submission() {
    let mut pending = Some(PendingSeek {
        request: 1,
        native_id: 101,
        acknowledged: false,
        started: false,
        snapshot: false,
        position: 300.0,
        checking_capability: false,
        // A successful capability reply has only marked this ready.
        submission_pending: true,
        deadline: Instant::now() + NATIVE_OPERATION_TIMEOUT,
    });
    let mut queued = Some((2, 400.0));
    let mut submitted = Vec::new();
    for event in [
        MPV_EVENT_END_FILE,
        MPV_EVENT_START_FILE,
        MPV_EVENT_FILE_LOADED,
        MPV_EVENT_NONE,
    ] {
        observe_seek_boundary(event, &mut pending, &mut queued);
        if event == MPV_EVENT_NONE {
            submitted.extend(take_seek_submission(&mut pending));
        }
    }
    assert!(
        submitted.is_empty(),
        "old media target must never reach new media"
    );
    assert!(queued.is_none());
}

#[test]
fn local_load_or_stop_invalidates_work_before_native_events() {
    let mut pending = Some(PendingSeek {
        request: 1,
        native_id: 101,
        acknowledged: false,
        started: false,
        snapshot: false,
        position: 300.0,
        checking_capability: true,
        submission_pending: false,
        deadline: Instant::now() + NATIVE_OPERATION_TIMEOUT,
    });
    let mut queued = Some((2, 400.0));
    invalidate_seeks(&mut pending, &mut queued);
    assert!(
        pending.is_none(),
        "late capability replies must have no matching request"
    );
    assert!(queued.is_none());
    assert!(take_seek_submission(&mut pending).is_none());
}
