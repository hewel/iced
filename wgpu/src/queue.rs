use std::marker::PhantomData;
use std::rc::Rc;

/// External, non-reentrant synchronization for a shared native graphics queue.
///
/// Use the same instance for iced and all other users of the native queue.
/// Queue writes only stage data until the next submission; iced guards actual
/// submissions, not drawing or primitive preparation. Custom submissions, raw
/// queue operations, and custom compositor surface operations must acquire a
/// [`QueueGuard`] themselves. Do not hold a guard across rendering, device
/// polling, thread joins, or calls into another native renderer (such as mpv).
///
/// Submission callbacks may run with the guard already held. They must be
/// CPU-only and must not reenter this gate or invoke native queue operations.
///
/// # Safety
/// Implementations must provide mutual exclusion, current-thread ownership,
/// and acquire/release memory visibility for every successful lock/unlock pair.
/// `lock` must not panic after acquiring ownership, and `unlock` must not panic.
/// The gate is non-reentrant: locking again on the owning thread is unsupported.
#[allow(unsafe_code)]
pub unsafe trait QueueSynchronization: Send + Sync {
    /// Acquires exclusive ownership on the current thread.
    fn lock(&self);

    /// Releases exclusive ownership on the current thread.
    ///
    /// # Safety
    /// The caller must own a successful, not-yet-released lock on this thread.
    unsafe fn unlock(&self);
}

/// An allocation-free queue lock that must be released on its acquiring thread.
///
/// This guard is neither `Send` nor `Sync`. Do not acquire it recursively or
/// retain it while calling an iced operation that acquires the same gate.
#[must_use = "dropping the guard immediately releases queue synchronization"]
pub struct QueueGuard<'a> {
    synchronization: &'a dyn QueueSynchronization,
    _thread: PhantomData<Rc<()>>,
}

impl<'a> QueueGuard<'a> {
    /// Acquires the gate, constructing a guard only after locking succeeds.
    pub fn acquire(synchronization: &'a dyn QueueSynchronization) -> Self {
        synchronization.lock();

        Self {
            synchronization,
            _thread: PhantomData,
        }
    }
}

impl Drop for QueueGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: acquire successfully locked before constructing this guard;
        // its private fields and !Send/!Sync marker preserve same-thread ownership.
        #[allow(unsafe_code)]
        unsafe {
            self.synchronization.unlock();
        }
    }
}
