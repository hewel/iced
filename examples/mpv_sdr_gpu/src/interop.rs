use crate::deadline::{NATIVE_OPERATION_TIMEOUT, WakeTimer};
use crate::{HostAbi, Settings, completion::Completion, gpu::DeviceContext, pool::Lease};
use ash::vk;
use iced_wgpu::wgpu;
use parking_lot::{Mutex, RawMutex, lock_api::RawMutex as _};
use std::{
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_int, c_void},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Debug, Clone)]
pub enum Command {
    TogglePause,
    /// Request IDs belong to the caller and must not be reused while pending.
    /// Seeks are serialized; only the latest queued target is retained.
    /// Superseded queued requests do not emit SeekFinished. File transitions
    /// discard all pending requests. Do not race these with external IPC seeks.
    SeekAbsolute {
        request: u64,
        position: f64,
    },
    SeekRelative(f64),
    SetVolume(f64),
    Stop,
    LoadFile(String),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    TimePos(Option<f64>),
    Duration(Option<f64>),
    Pause(Option<bool>),
    Volume(Option<f64>),
    Idle(Option<bool>),
    Seeking(Option<bool>),
    Buffering(Option<bool>),
    Seekable(Option<bool>),
    StartFile,
    FileLoaded,
    /// Loaded source snapshot; emitted even when the same path is reloaded.
    MediaPath(Option<String>),
    PlaybackRestart,
    /// Snapshot after this request's acknowledged seek/restart, not a proximity
    /// test against the target. Sparse media may land beyond the requested time.
    SeekFinished {
        request: u64,
        result: Result<f64, String>,
    },
    EndFile {
        error: Option<String>,
    },
    Error(String),
    Shutdown,
}

// client.h enum values use C integers, not Rust enums: future IDs are valid.
const MPV_FORMAT_NONE: c_int = 0;
const MPV_FORMAT_FLAG: c_int = 3;
const MPV_FORMAT_DOUBLE: c_int = 5;
const MPV_EVENT_NONE: c_int = 0;
const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_LOG_MESSAGE: c_int = 2;
const MPV_EVENT_GET_PROPERTY_REPLY: c_int = 3;
const MPV_EVENT_COMMAND_REPLY: c_int = 5;
const MPV_EVENT_START_FILE: c_int = 6;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_FILE_LOADED: c_int = 8;
const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;
const MPV_EVENT_SEEK: c_int = 20;
const MPV_EVENT_PLAYBACK_RESTART: c_int = 21;
const MPV_EVENT_QUEUE_OVERFLOW: c_int = 24;
const MPV_END_FILE_REASON_ERROR: c_int = 4;
const MPV_LOG_LEVEL_ERROR: c_int = 20;

#[repr(C)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventLogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}

#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

// ABI layouts match mpv/include/mpv/gpu_next.h. Older versions only read their prefix.
#[repr(C)]
#[derive(Clone, Copy)]
struct Target {
    image: vk::Image,
    width: c_int,
    height: c_int,
    layout: vk::ImageLayout,
    usage: vk::ImageUsageFlags,
    token: u64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Frame {
    target: Target,
    semaphore: vk::Semaphore,
    wait_value: u64,
    signal_value: u64,
}
#[repr(C)]
struct TargetColor {
    primaries: c_int,
    transfer: c_int,
    ref_luma: f32,
    min_luma: f32,
    max_luma: f32,
    depth: c_int,
}
#[repr(C)]
struct Descriptor {
    version: u32,
    instance: vk::Instance,
    physical_device: vk::PhysicalDevice,
    device: vk::Device,
    get_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    queue_family: u32,
    features: *const vk::PhysicalDeviceFeatures2<'static>,
    extensions: *const *const c_char,
    num_extensions: c_int,
    opaque: *mut c_void,
    lock_queue: unsafe extern "C" fn(*mut c_void, u32, u32),
    unlock_queue: unsafe extern "C" fn(*mut c_void, u32, u32),
    acquire: unsafe extern "C" fn(*mut c_void, *mut Target) -> c_int,
    release: unsafe extern "C" fn(*mut c_void, *const Target, c_int),
    target_color: Option<unsafe extern "C" fn(*mut c_void, *mut TargetColor)>,
    acquire_frame: Option<unsafe extern "C" fn(*mut c_void, *mut Frame) -> c_int>,
    release_frame: Option<unsafe extern "C" fn(*mut c_void, *const Frame, c_int)>,
}

pub struct QueueLock(RawMutex);
impl QueueLock {
    fn new() -> Self {
        Self(RawMutex::INIT)
    }
    pub fn lock(&self) -> iced_wgpu::QueueGuard<'_> {
        iced_wgpu::QueueGuard::acquire(self)
    }
}
// SAFETY: RawMutex provides same-thread, non-reentrant acquire/release exclusion.
unsafe impl iced_wgpu::QueueSynchronization for QueueLock {
    fn lock(&self) {
        self.0.lock();
    }
    unsafe fn unlock(&self) {
        unsafe { self.0.unlock() };
    }
}

struct Slot {
    target: Target,
    memory: vk::DeviceMemory,
    lease: Lease,
    semaphore: vk::Semaphore,
}
struct Pool {
    slots: Vec<Slot>,
    ready: VecDeque<usize>,
    stopping: bool,
    size: (u32, u32),
    error: Option<String>,
}
struct Shared {
    raw: ash::Device,
    _device: wgpu::Device,
    asynchronous: bool,
    memory: vk::PhysicalDeviceMemoryProperties,
    family: u32,
    queue_lock: Arc<QueueLock>,
    pool: Mutex<Pool>,
    exhausted: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

// The HAL attachments are one-shot queue state. Remove unconsumed entries even
// if submission validation unwinds, before the enclosing queue gate is released.
struct SubmissionSync<'a> {
    queue: &'a wgpu::Queue,
    semaphore: vk::Semaphore,
}
impl Drop for SubmissionSync<'_> {
    fn drop(&mut self) {
        if let Some(native) = unsafe { self.queue.as_hal::<wgpu::hal::api::Vulkan>() } {
            native.remove_wait_semaphore(self.semaphore);
            native.remove_signal_semaphore(self.semaphore);
        }
    }
}
impl Shared {
    fn allocate(
        &self,
        width: u32,
        height: u32,
        token: u64,
    ) -> Result<Slot, Box<dyn std::error::Error>> {
        unsafe {
            let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC;
            let image = self.raw.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::A2B10G10R10_UNORM_PACK32)
                    .extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let requirements = self.raw.get_image_memory_requirements(image);
            let memory_type = (0..self.memory.memory_type_count).find(|&i| {
                requirements.memory_type_bits & (1 << i) != 0
                    && self.memory.memory_types[i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            });
            let Some(memory_type) = memory_type else {
                self.raw.destroy_image(image, None);
                return Err("No device-local Vulkan image memory".into());
            };
            let memory = match self.raw.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            ) {
                Ok(memory) => memory,
                Err(error) => {
                    self.raw.destroy_image(image, None);
                    return Err(error.into());
                }
            };
            if let Err(error) = self.raw.bind_image_memory(image, memory, 0) {
                self.raw.destroy_image(image, None);
                self.raw.free_memory(memory, None);
                return Err(error.into());
            }
            Ok(Slot {
                target: Target {
                    image,
                    width: width as i32,
                    height: height as i32,
                    layout: vk::ImageLayout::UNDEFINED,
                    usage,
                    token,
                },
                memory,
                lease: Lease::default(),
                semaphore: vk::Semaphore::null(),
            })
        }
    }
}
impl Drop for Shared {
    fn drop(&mut self) {
        for slot in &self.pool.get_mut().slots {
            unsafe {
                self.raw.destroy_image(slot.target.image, None);
                self.raw.free_memory(slot.memory, None);
                self.raw.destroy_semaphore(slot.semaphore, None);
            }
        }
    }
}
unsafe extern "C" fn lock_queue(opaque: *mut c_void, family: u32, index: u32) {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    // A mismatch is an ABI violation; never unwind through C.
    if family != shared.family || index != 0 {
        std::process::abort();
    }
    shared.queue_lock.0.lock();
}
unsafe extern "C" fn unlock_queue(opaque: *mut c_void, family: u32, index: u32) {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    if family != shared.family || index != 0 {
        std::process::abort();
    }
    unsafe { shared.queue_lock.0.unlock() };
}
unsafe extern "C" fn acquire(opaque: *mut c_void, out: *mut Target) -> c_int {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    match shared.acquire() {
        Ok(Some(frame)) => {
            unsafe { out.write(frame.target) };
            1
        }
        Ok(None) => 0,
        Err(()) => -1,
    }
}
unsafe extern "C" fn acquire_frame(opaque: *mut c_void, out: *mut Frame) -> c_int {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    match shared.acquire() {
        Ok(Some(frame)) => {
            unsafe { out.write(frame) };
            1
        }
        Ok(None) => 0,
        Err(()) => -1,
    }
}
unsafe extern "C" fn release(opaque: *mut c_void, target: *const Target, status: c_int) {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    shared.release(unsafe { (*target).token }, status);
}
unsafe extern "C" fn release_frame(opaque: *mut c_void, frame: *const Frame, status: c_int) {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    shared.release(unsafe { (*frame).target.token }, status);
}
unsafe extern "C" fn wakeup(opaque: *mut c_void) {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    (shared.wake)();
}
impl Shared {
    fn acquire(&self) -> Result<Option<Frame>, ()> {
        let Some(mut pool) = self.pool.try_lock() else {
            self.exhausted.store(true, Ordering::Release);
            // The UI may have just released capacity before this contention.
            // Always wake it to retry; there is no periodic timer to rescue us.
            (self.wake)();
            return Ok(None);
        };
        if pool.stopping {
            return Ok(None);
        }
        let size = pool.size;
        let Some(slot) = pool.slots.iter_mut().find(|slot| {
            slot.lease.is_idle() && (slot.target.width as u32, slot.target.height as u32) == size
        }) else {
            self.exhausted.store(true, Ordering::Release);
            return Ok(None);
        };
        let Some((wait_value, signal_value)) = slot.lease.acquire() else {
            pool.error = Some("host timeline exhausted".into());
            pool.stopping = true;
            drop(pool);
            (self.wake)();
            return Err(());
        };
        Ok(Some(Frame {
            target: slot.target,
            semaphore: slot.semaphore,
            wait_value,
            signal_value,
        }))
    }

    fn release(&self, token: u64, status: c_int) {
        let mut pool = self.pool.lock();
        let Some(slot) = pool.slots.get_mut(token as usize) else {
            std::process::abort()
        };
        if !slot.lease.release(status == 0) {
            std::process::abort();
        }
        if status == 0 {
            slot.target.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
            pool.ready.push_back(token as usize);
        } else {
            // No producer signal is promised on error. Quarantine this session;
            // never recycle the image or wait on a potentially unsignalled value.
            slot.target.layout = vk::ImageLayout::UNDEFINED;
            if !pool.stopping {
                pool.error = Some(format!(
                    "mpv discarded a GPU target ({status}); host session stopped"
                ));
            }
            pool.stopping = true;
        }
        drop(pool);
        (self.wake)();
    }
}

struct PendingSeek {
    request: u64,
    native_id: u64,
    acknowledged: bool,
    started: bool,
    snapshot: bool,
    position: f64,
    checking_capability: bool,
    submission_pending: bool,
    deadline: Instant,
}

fn observe_seek_boundary(
    event_id: c_int,
    pending: &mut Option<PendingSeek>,
    queued: &mut Option<(u64, f64)>,
) {
    if matches!(event_id, MPV_EVENT_END_FILE | MPV_EVENT_START_FILE) {
        invalidate_seeks(pending, queued);
    }
}

fn invalidate_seeks(pending: &mut Option<PendingSeek>, queued: &mut Option<(u64, f64)>) {
    *pending = None;
    *queued = None;
}

fn take_seek_submission(pending: &mut Option<PendingSeek>) -> Option<(u64, u64, f64)> {
    let seek = pending.as_mut().filter(|seek| seek.submission_pending)?;
    seek.submission_pending = false;
    Some((seek.request, seek.native_id, seek.position))
}

const SEEK_TIMEOUT_ERROR: &str =
    "seek completion timed out; seeking disabled until media reload (native seek may still finish)";

fn expire_seek(
    pending: &mut Option<PendingSeek>,
    queued: &mut Option<(u64, f64)>,
    failed: &mut bool,
    now: Instant,
    events: &mut Vec<PlayerEvent>,
) {
    if pending.as_ref().is_some_and(|seek| now >= seek.deadline) {
        *failed = true;
        let request = pending.take().expect("expired seek").request;
        events.push(PlayerEvent::SeekFinished {
            request,
            result: Err(SEEK_TIMEOUT_ERROR.into()),
        });
        if let Some((request, _)) = queued.take() {
            events.push(PlayerEvent::SeekFinished {
                request,
                result: Err(SEEK_TIMEOUT_ERROR.into()),
            });
        }
    }
}

pub struct Host {
    handle: *mut c_void,
    terminate: unsafe extern "C" fn(*mut c_void),
    request_redraw: unsafe extern "C" fn(*mut c_void) -> c_int,
    command_async: unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent,
    get_property_async: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    _library: libloading::Library,
    _descriptor: Box<Descriptor>,
    _extensions: Vec<*const c_char>,
    shared: Arc<Shared>,
    completion: Completion,
    context: Rc<DeviceContext>,
    queued_seek: Option<(u64, f64)>,
    seek: Option<PendingSeek>,
    next_seek_id: u64,
    media_id: u64,
    seek_failed: bool,
    seek_timer: WakeTimer,
}
impl Host {
    pub fn new(
        context: Rc<DeviceContext>,
        options: &Settings,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            let native = context
                .device
                .as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or("Vulkan required")?;
            if native.queue_index() != 0 {
                return Err("mpv requires queue index zero".into());
            }
            let instance = native.shared_instance();
            let memory = instance
                .raw_instance()
                .get_physical_device_memory_properties(native.raw_physical_device());
            let shared = Arc::new(Shared {
                raw: native.raw_device().clone(),
                _device: context.device.clone(),
                asynchronous: options.host_abi == HostAbi::V3,
                family: native.queue_family_index(),
                memory,
                queue_lock: Arc::new(QueueLock::new()),
                pool: Mutex::new(Pool {
                    slots: Vec::new(),
                    ready: VecDeque::new(),
                    stopping: false,
                    size: (options.width, options.height),
                    error: None,
                }),
                exhausted: AtomicBool::new(false),
                wake: Box::new(wake),
            });
            for index in 0..3 {
                let slot = shared.allocate(options.width, options.height, index)?;
                shared.pool.lock().slots.push(slot);
                if shared.asynchronous {
                    let mut timeline = vk::SemaphoreTypeCreateInfo::default()
                        .semaphore_type(vk::SemaphoreType::TIMELINE)
                        .initial_value(0);
                    let semaphore = shared.raw.create_semaphore(
                        &vk::SemaphoreCreateInfo::default().push_next(&mut timeline),
                        None,
                    )?;
                    shared.pool.lock().slots[index as usize].semaphore = semaphore;
                }
            }
            let extensions: Vec<_> = native
                .enabled_device_extensions()
                .iter()
                .map(|e| e.as_ptr())
                .collect();
            let descriptor = Box::new(Descriptor {
                version: options.host_abi as u32,
                instance: instance.raw_instance().handle(),
                physical_device: native.raw_physical_device(),
                device: shared.raw.handle(),
                get_proc_addr: instance.entry().static_fn().get_instance_proc_addr,
                queue_family: shared.family,
                features: context.features(),
                extensions: extensions.as_ptr(),
                num_extensions: extensions.len() as i32,
                opaque: Arc::as_ptr(&shared).cast_mut().cast(),
                lock_queue,
                unlock_queue,
                acquire,
                release,
                target_color: None,
                acquire_frame: shared.asynchronous.then_some(acquire_frame),
                release_frame: shared.asynchronous.then_some(release_frame),
            });
            // Do not retain a wgpu internal guard across libmpv initialization
            // or fallible teardown paths that poll the same device.
            drop(native);
            let library = libloading::Library::new(&options.libmpv)?;
            let create = *library.get::<unsafe extern "C" fn() -> *mut c_void>(b"mpv_create\0")?;
            let terminate =
                *library.get::<unsafe extern "C" fn(*mut c_void)>(b"mpv_terminate_destroy\0")?;
            let request_redraw = *library.get::<unsafe extern "C" fn(*mut c_void) -> c_int>(
                b"mpv_gpu_next_request_redraw\0",
            )?;
            let set_host = *library
                .get::<unsafe extern "C" fn(*mut c_void, *const Descriptor) -> c_int>(
                    b"mpv_gpu_next_set_host\0",
                )?;
            let set_option = *library.get::<unsafe extern "C" fn(
                *mut c_void,
                *const c_char,
                *const c_char,
            ) -> c_int>(b"mpv_set_option_string\0")?;
            let initialize =
                *library.get::<unsafe extern "C" fn(*mut c_void) -> c_int>(b"mpv_initialize\0")?;
            let command = *library
                .get::<unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int>(
                    b"mpv_command\0",
                )?;
            let command_async =
                *library
                    .get::<unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> c_int>(
                        b"mpv_command_async\0",
                    )?;
            let observe_property =
                *library
                    .get::<unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int>(
                        b"mpv_observe_property\0",
                    )?;
            let get_property_async =
                *library
                    .get::<unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int>(
                        b"mpv_get_property_async\0",
                    )?;
            let wait_event = *library
                .get::<unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent>(
                    b"mpv_wait_event\0",
                )?;
            let error_string = *library
                .get::<unsafe extern "C" fn(c_int) -> *const c_char>(b"mpv_error_string\0")?;
            let request_log_messages =
                *library.get::<unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int>(
                    b"mpv_request_log_messages\0",
                )?;
            let set_wakeup = *library.get::<unsafe extern "C" fn(
                *mut c_void,
                Option<unsafe extern "C" fn(*mut c_void)>,
                *mut c_void,
            )>(b"mpv_set_wakeup_callback\0")?;
            let completed = shared.clone();
            let completion = Completion::new(context.device.clone(), move |token, result| {
                let mut pool = completed.pool.lock();
                match result {
                    Ok(()) => {
                        let slot = &mut pool.slots[token];
                        if !slot.lease.complete() {
                            std::process::abort();
                        }
                        slot.target.layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
                    }
                    Err(error) => {
                        pool.error = Some(error);
                        pool.stopping = true;
                    }
                }
                let needs_ui = pool.error.is_some()
                    || completed.exhausted.load(Ordering::Acquire)
                    || pool.slots.iter().any(|slot| {
                        slot.lease.is_idle()
                            && (slot.target.width as u32, slot.target.height as u32) != pool.size
                    });
                drop(pool);
                if needs_ui {
                    (completed.wake)();
                }
            })?;
            let timer_shared = shared.clone();
            let seek_timer = WakeTimer::new(move || (timer_shared.wake)())?;
            let handle = create();
            if handle.is_null() {
                return Err("mpv_create failed".into());
            }
            let host = Self {
                handle,
                terminate,
                request_redraw,
                command_async,
                wait_event,
                get_property_async,
                error_string,
                _library: library,
                _descriptor: descriptor,
                _extensions: extensions,
                shared,
                completion,
                context,
                queued_seek: None,
                seek: None,
                next_seek_id: 1 << 63,
                media_id: 10,
                seek_failed: false,
                seek_timer,
            };
            set_wakeup(
                handle,
                Some(wakeup),
                Arc::as_ptr(&host.shared).cast_mut().cast(),
            );
            let option = |name: &str, value: &str| -> Result<(), Box<dyn std::error::Error>> {
                let name = CString::new(name)?;
                let value = CString::new(value)?;
                let result = set_option(handle, name.as_ptr(), value.as_ptr());
                if result < 0 {
                    return Err(format!("mpv option {name:?} failed: {result}").into());
                }
                Ok(())
            };
            option("config", "no")?;
            option("include", &options.baseline)?;
            option("terminal", "yes")?;
            option("start", &options.start.to_string())?;
            option("pause", if options.pause { "yes" } else { "no" })?;
            if let Some(ipc) = &options.ipc {
                option("input-ipc-server", ipc)?;
            }
            let result = set_host(handle, &*host._descriptor);
            if result < 0 {
                return Err(format!("mpv_gpu_next_set_host failed: {result}").into());
            }
            let result = initialize(handle);
            if result < 0 {
                return Err(format!("mpv_initialize failed: {result}").into());
            }
            let result = request_log_messages(handle, c"error".as_ptr());
            if result < 0 {
                return Err(format!("mpv log subscription failed: {}", host.error(result)).into());
            }
            for (id, name, format) in [
                (1, c"time-pos", MPV_FORMAT_DOUBLE),
                (2, c"duration", MPV_FORMAT_DOUBLE),
                (3, c"pause", MPV_FORMAT_FLAG),
                (4, c"volume", MPV_FORMAT_DOUBLE),
                (5, c"idle-active", MPV_FORMAT_FLAG),
                (6, c"seeking", MPV_FORMAT_FLAG),
                (7, c"paused-for-cache", MPV_FORMAT_FLAG),
                (8, c"seekable", MPV_FORMAT_FLAG),
            ] {
                let result = observe_property(handle, id, name.as_ptr(), format);
                if result < 0 {
                    return Err(
                        format!("mpv observe {name:?} failed: {}", host.error(result)).into(),
                    );
                }
            }
            let load = CString::new("loadfile")?;
            let source = CString::new(options.source.as_str())?;
            let args = [load.as_ptr(), source.as_ptr(), std::ptr::null()];
            let result = command(handle, args.as_ptr());
            if result < 0 {
                return Err(format!("loadfile failed: {result}").into());
            }
            Ok(host)
        }
    }
    pub fn queue_lock(&self) -> Arc<QueueLock> {
        self.shared.queue_lock.clone()
    }

    pub fn fail(&self, error: String) {
        let mut pool = self.shared.pool.lock();
        pool.error = Some(error);
        pool.stopping = true;
        drop(pool);
        (self.shared.wake)();
    }

    fn error(&self, status: c_int) -> String {
        // mpv_error_string returns static storage, never owned by the caller.
        unsafe { CStr::from_ptr((self.error_string)(status)) }
            .to_string_lossy()
            .into_owned()
    }

    /// Application-thread only, outside the queue lock and every VO callback.
    /// Success means queued; execution failures arrive through poll_events.
    pub fn execute(&mut self, command: Command) -> Result<(), String> {
        if matches!(command, Command::LoadFile(_) | Command::Stop) {
            // Invalidate at admission, not only when native lifecycle events
            // arrive: an old capability reply may already be queued.
            invalidate_seeks(&mut self.seek, &mut self.queued_seek);
            self.seek_timer.arm(None);
        }
        if let Command::SeekAbsolute { request, position } = command {
            if self.seek_failed {
                return Err(SEEK_TIMEOUT_ERROR.into());
            }
            if !position.is_finite() || position < 0.0 {
                return Err("seek position must be finite and nonnegative".into());
            }
            self.queued_seek = Some((request, position));
            (self.shared.wake)();
            return Ok(());
        }
        let (args, id): (Vec<String>, u64) = match command {
            Command::TogglePause => (vec!["cycle".into(), "pause".into()], 1),
            Command::Stop => (vec!["stop".into()], 5),
            Command::LoadFile(source) => (vec!["loadfile".into(), source], 6),
            Command::SeekAbsolute { .. } => unreachable!(),
            Command::SeekRelative(value) | Command::SetVolume(value) => {
                if !value.is_finite() {
                    return Err("mpv command requires a finite value".into());
                }
                if matches!(command, Command::SetVolume(_)) && value < 0.0 {
                    return Err("mpv absolute position and volume must be nonnegative".into());
                }
                let value = value.to_string();
                match command {
                    Command::SeekRelative(_) => {
                        (vec!["seek".into(), value, "relative+exact".into()], 3)
                    }
                    Command::SetVolume(_) => (vec!["set".into(), "volume".into(), value], 4),
                    _ => unreachable!(),
                }
            }
        };
        let args = args
            .into_iter()
            .map(CString::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let mut pointers: Vec<_> = args.iter().map(|arg| arg.as_ptr()).collect();
        pointers.push(std::ptr::null());
        // mpv copies command arguments before returning.
        let status = unsafe { (self.command_async)(self.handle, id, pointers.as_ptr()) };
        if status < 0 {
            Err(format!("mpv command failed: {}", self.error(status)))
        } else {
            Ok(())
        }
    }

    fn begin_seek(&mut self, request: u64, position: f64) -> Result<(), String> {
        let native_id = self.next_seek_id;
        self.next_seek_id = native_id.checked_add(1).ok_or("seek ID exhausted")?;
        let status = unsafe {
            (self.get_property_async)(
                self.handle,
                native_id,
                c"seekable".as_ptr(),
                MPV_FORMAT_FLAG,
            )
        };
        if status < 0 {
            return Err(self.error(status));
        }
        self.seek = Some(PendingSeek {
            request,
            native_id,
            acknowledged: false,
            started: false,
            snapshot: false,
            position,
            checking_capability: true,
            submission_pending: false,
            deadline: Instant::now() + NATIVE_OPERATION_TIMEOUT,
        });
        self.seek_timer
            .arm(self.seek.as_ref().map(|seek| seek.deadline));
        Ok(())
    }

    fn submit_seek(&self, native_id: u64, position: f64) -> Result<(), String> {
        let position = CString::new(position.to_string()).expect("finite number");
        let args = [
            c"seek".as_ptr(),
            position.as_ptr(),
            c"absolute+exact".as_ptr(),
            std::ptr::null(),
        ];
        let status = unsafe { (self.command_async)(self.handle, native_id, args.as_ptr()) };
        if status < 0 {
            Err(self.error(status))
        } else {
            Ok(())
        }
    }

    /// Application-thread only, outside the queue lock and every VO callback.
    /// All payloads are copied before the next wait_event invalidates them.
    pub fn poll_events(&mut self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        if let Some(error) = self.shared.pool.lock().error.take() {
            events.push(PlayerEvent::Error(error));
            events.push(PlayerEvent::Shutdown);
        }
        // A successful command reply is only queue acceptance. If no restart
        // follows, fail closed for this media: late untagged SEEK/restart events
        // cannot safely be attributed to another submission.
        expire_seek(
            &mut self.seek,
            &mut self.queued_seek,
            &mut self.seek_failed,
            Instant::now(),
            &mut events,
        );
        // Yield to the application during bursts, scheduling another turn even
        // if every event in this batch was an ignored future event type.
        for _ in 0..256 {
            let event = unsafe { &*(self.wait_event)(self.handle, 0.0) };
            observe_seek_boundary(event.event_id, &mut self.seek, &mut self.queued_seek);
            match event.event_id {
                MPV_EVENT_NONE => {
                    // Only submit after all already-queued lifecycle events
                    // have had a chance to invalidate the capability result.
                    if let Some((request, native_id, position)) =
                        take_seek_submission(&mut self.seek)
                        && let Err(error) = self.submit_seek(native_id, position)
                    {
                        self.seek = None;
                        events.push(PlayerEvent::SeekFinished {
                            request,
                            result: Err(error),
                        });
                    }
                    if self.seek.is_none()
                        && let Some((request, position)) = self.queued_seek.take()
                        && let Err(error) = self.begin_seek(request, position)
                    {
                        events.push(PlayerEvent::SeekFinished {
                            request,
                            result: Err(error),
                        });
                    }
                    self.seek_timer
                        .arm(self.seek.as_ref().map(|seek| seek.deadline));
                    return events;
                }
                MPV_EVENT_SHUTDOWN => events.push(PlayerEvent::Shutdown),
                MPV_EVENT_START_FILE => {
                    self.seek_failed = false;
                    self.media_id = self.media_id.checked_add(1).expect("media IDs exhausted");
                    events.push(PlayerEvent::StartFile);
                }
                MPV_EVENT_FILE_LOADED => {
                    events.push(PlayerEvent::FileLoaded);
                    let status = unsafe {
                        (self.get_property_async)(self.handle, self.media_id, c"path".as_ptr(), 1)
                    };
                    if status < 0 {
                        events.push(PlayerEvent::MediaPath(None));
                    }
                }
                MPV_EVENT_GET_PROPERTY_REPLY if event.reply_userdata == self.media_id => {
                    let path = if event.error >= 0 && !event.data.is_null() {
                        let property = unsafe { &*event.data.cast::<MpvEventProperty>() };
                        if property.format == 1 && !property.data.is_null() {
                            let pointer = unsafe { *property.data.cast::<*const c_char>() };
                            if pointer.is_null() {
                                None
                            } else {
                                Some(
                                    unsafe { CStr::from_ptr(pointer) }
                                        .to_string_lossy()
                                        .into_owned(),
                                )
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    events.push(PlayerEvent::MediaPath(path));
                }
                MPV_EVENT_SEEK => {
                    if let Some(seek) = &mut self.seek
                        && seek.acknowledged
                    {
                        seek.started = true;
                    }
                }
                MPV_EVENT_PLAYBACK_RESTART => {
                    events.push(PlayerEvent::PlaybackRestart);
                    if let Some(seek) = &mut self.seek
                        && seek.started
                        && !seek.snapshot
                    {
                        seek.snapshot = true;
                        let status = unsafe {
                            (self.get_property_async)(
                                self.handle,
                                seek.native_id,
                                c"time-pos".as_ptr(),
                                MPV_FORMAT_DOUBLE,
                            )
                        };
                        if status < 0 {
                            let request = seek.request;
                            self.seek = None;
                            events.push(PlayerEvent::SeekFinished {
                                request,
                                result: Err(self.error(status)),
                            });
                        }
                    }
                }
                MPV_EVENT_GET_PROPERTY_REPLY
                    if self.seek.as_ref().is_some_and(|seek| {
                        seek.checking_capability && seek.native_id == event.reply_userdata
                    }) =>
                {
                    let seek = self.seek.as_mut().expect("matched capability query");
                    seek.checking_capability = false;
                    let request = seek.request;
                    let seekable = if event.error >= 0 && !event.data.is_null() {
                        let property = unsafe { &*event.data.cast::<MpvEventProperty>() };
                        property.format == MPV_FORMAT_FLAG
                            && !property.data.is_null()
                            && unsafe { *property.data.cast::<c_int>() } != 0
                    } else {
                        false
                    };
                    if seekable {
                        seek.submission_pending = true;
                    } else {
                        let error =
                            "source is not seekable or seek capability is unavailable".into();
                        self.seek = None;
                        events.push(PlayerEvent::SeekFinished {
                            request,
                            result: Err(error),
                        });
                    }
                }
                MPV_EVENT_GET_PROPERTY_REPLY
                    if self.seek.as_ref().is_some_and(|seek| {
                        seek.snapshot && seek.native_id == event.reply_userdata
                    }) =>
                {
                    let seek = self.seek.take().expect("matched seek");
                    let result = if event.error < 0 {
                        Err(self.error(event.error))
                    } else if event.data.is_null() {
                        Err("seek completed without a position".into())
                    } else {
                        let property = unsafe { &*event.data.cast::<MpvEventProperty>() };
                        if property.format == MPV_FORMAT_DOUBLE && !property.data.is_null() {
                            let position = unsafe { *property.data.cast::<f64>() };
                            if position.is_finite() {
                                Ok(position)
                            } else {
                                Err("invalid seek position".into())
                            }
                        } else {
                            Err("seek position unavailable".into())
                        }
                    };
                    events.push(PlayerEvent::SeekFinished {
                        request: seek.request,
                        result,
                    });
                }
                MPV_EVENT_COMMAND_REPLY
                    if self
                        .seek
                        .as_ref()
                        .is_some_and(|seek| seek.native_id == event.reply_userdata) =>
                {
                    if event.error < 0 {
                        let seek = self.seek.take().expect("matched seek");
                        events.push(PlayerEvent::SeekFinished {
                            request: seek.request,
                            result: Err(self.error(event.error)),
                        });
                    } else if let Some(seek) = &mut self.seek {
                        seek.acknowledged = true;
                    }
                }
                MPV_EVENT_END_FILE if !event.data.is_null() => {
                    self.seek_failed = false;
                    self.media_id = self.media_id.checked_add(1).expect("media IDs exhausted");
                    let end = unsafe { &*event.data.cast::<MpvEventEndFile>() };
                    let error =
                        (end.reason == MPV_END_FILE_REASON_ERROR).then(|| self.error(end.error));
                    events.push(PlayerEvent::EndFile { error });
                }
                MPV_EVENT_COMMAND_REPLY if event.error < 0 => {
                    let command = match event.reply_userdata {
                        1 => "cycle pause",
                        2 => "absolute seek",
                        3 => "relative seek",
                        4 => "set volume",
                        5 => "stop",
                        6 => "loadfile",
                        _ => "command",
                    };
                    events.push(PlayerEvent::Error(format!(
                        "mpv {command} failed: {}",
                        self.error(event.error)
                    )));
                }
                MPV_EVENT_QUEUE_OVERFLOW => events.push(PlayerEvent::Error(
                    "mpv event queue overflow: player updates were lost".into(),
                )),
                MPV_EVENT_LOG_MESSAGE if !event.data.is_null() => {
                    let log = unsafe { &*event.data.cast::<MpvEventLogMessage>() };
                    if !log.prefix.is_null() && !log.text.is_null() {
                        let prefix = unsafe { CStr::from_ptr(log.prefix) };
                        if log.log_level <= MPV_LOG_LEVEL_ERROR || prefix == c"overflow" {
                            let text = unsafe { CStr::from_ptr(log.text) }.to_string_lossy();
                            events.push(PlayerEvent::Error(format!(
                                "mpv [{}]: {}",
                                prefix.to_string_lossy(),
                                text.trim_end()
                            )));
                        }
                    }
                }
                MPV_EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                    let property = unsafe { &*event.data.cast::<MpvEventProperty>() };
                    // NONE makes data invalid; never dereference it.
                    let available = property.format != MPV_FORMAT_NONE && !property.data.is_null();
                    let number = if available && property.format == MPV_FORMAT_DOUBLE {
                        let value = unsafe { *property.data.cast::<f64>() };
                        value.is_finite().then_some(value)
                    } else {
                        None
                    };
                    let flag = if available && property.format == MPV_FORMAT_FLAG {
                        Some(unsafe { *property.data.cast::<c_int>() } != 0)
                    } else {
                        None
                    };
                    let property = match event.reply_userdata {
                        1 => PlayerEvent::TimePos(number),
                        2 => PlayerEvent::Duration(number),
                        3 => PlayerEvent::Pause(flag),
                        4 => PlayerEvent::Volume(number),
                        5 => PlayerEvent::Idle(flag),
                        6 => PlayerEvent::Seeking(flag),
                        7 => PlayerEvent::Buffering(flag),
                        8 => PlayerEvent::Seekable(flag),
                        _ => continue,
                    };
                    events.push(property);
                }
                _ => {}
            }
        }
        (self.shared.wake)();
        events
    }

    /// Application-thread only, outside the queue lock and every VO callback.
    pub fn retry_if_capacity(&self) -> Result<(), String> {
        self.refresh_slots().map_err(|error| error.to_string())?;
        let retry = {
            let pool = self.shared.pool.lock();
            !pool.stopping
                && pool.slots.iter().any(|slot| {
                    slot.lease.is_idle()
                        && (slot.target.width as u32, slot.target.height as u32) == pool.size
                })
                && self.shared.exhausted.swap(false, Ordering::AcqRel)
        };
        if retry {
            self.request_redraw()?;
        }
        Ok(())
    }

    pub fn request_redraw(&self) -> Result<(), String> {
        let status = unsafe { (self.request_redraw)(self.handle) };
        if status < 0 {
            Err(format!("mpv_gpu_next_request_redraw failed: {status}"))
        } else {
            Ok(())
        }
    }

    pub fn resize(&self, width: u32, height: u32) -> Result<(), String> {
        {
            let mut pool = self.shared.pool.lock();
            if pool.size == (width, height) {
                return Ok(());
            }
            pool.size = (width, height);
        }
        self.refresh_slots().map_err(|error| error.to_string())?;
        self.request_redraw()
    }

    fn refresh_slots(&self) -> Result<(), Box<dyn std::error::Error>> {
        for index in 0..3 {
            let size = {
                let mut pool = self.shared.pool.lock();
                if pool.stopping {
                    return Ok(());
                }
                let size = pool.size;
                let slot = &mut pool.slots[index];
                if !slot.lease.is_idle()
                    || (slot.target.width as u32, slot.target.height as u32) == size
                {
                    continue;
                }
                // Reserve only GPU-idle slots. Allocation never holds the pool
                // lock, so VO callbacks cannot block behind a Vulkan allocation.
                slot.lease.reserve();
                size
            };
            let replacement = match self.shared.allocate(size.0, size.1, index as u64) {
                Ok(slot) => slot,
                Err(error) => {
                    self.shared.pool.lock().slots[index]
                        .lease
                        .cancel_reservation();
                    return Err(error);
                }
            };
            let old = {
                let mut pool = self.shared.pool.lock();
                let slot = &mut pool.slots[index];
                let mut replacement = replacement;
                replacement.semaphore = slot.semaphore;
                slot.lease.cancel_reservation();
                replacement.lease = slot.lease;
                std::mem::replace(slot, replacement)
            };
            unsafe {
                self.shared.raw.destroy_image(old.target.image, None);
                self.shared.raw.free_memory(old.memory, None);
            }
        }
        Ok(())
    }

    pub fn private_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("private 10-bit current SDR frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        // Tell wgpu this texture is initialized before writing via raw Vulkan.
        // Otherwise its lazy first-use clear can erase the native GPU copy.
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("initialize private SDR texture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        queue.submit([encoder.finish()]);
        texture
    }

    // Only successful scheduled releases enter this path. The v3 wait gates
    // the source transition and copy, independently of the CPU release callback.
    pub fn copy_ready(&self, queue: &wgpu::Queue, destination: &mut wgpu::Texture) -> Option<bool> {
        let (target, semaphore, signal_value) = {
            let mut pool = self.shared.pool.lock();
            if pool.stopping {
                return None;
            }
            let index = pool.ready.pop_front()?;
            let slot = &mut pool.slots[index];
            let signal = slot.lease.copy().expect("released target");
            (slot.target, slot.semaphore, signal)
        };
        let resized = destination.width() != target.width as u32
            || destination.height() != target.height as u32;
        if resized {
            let _guard = self.shared.queue_lock.lock();
            *destination = Self::private_texture(
                &self.context.device,
                queue,
                target.width as u32,
                target.height as u32,
            );
        }
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("scheduled SDR copy"),
                });
        encoder.transition_resources(
            std::iter::empty(),
            std::iter::once(wgpu::TextureTransition {
                texture: &*destination,
                selector: None,
                state: wgpu::TextureUses::COPY_DST,
            }),
        );
        let before = encoder.finish();
        // wgpu forbids mixing its encoder API with raw HAL recording.
        // Keep tracked transitions in separate command buffers around the copy.
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("raw SDR image copy"),
                });
        unsafe {
            let dest = destination
                .as_hal::<wgpu::hal::api::Vulkan>()
                .expect("Vulkan texture");
            let image = dest.raw_handle();
            encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|native| {
                let command = native.expect("Vulkan encoder").raw_handle();
                let range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1);
                let barrier = vk::ImageMemoryBarrier::default()
                    .image(target.image)
                    .subresource_range(range)
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
                self.shared.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
                let layers = vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .layer_count(1);
                let copy = vk::ImageCopy::default()
                    .src_subresource(layers)
                    .dst_subresource(layers)
                    .extent(vk::Extent3D {
                        width: target.width as u32,
                        height: target.height as u32,
                        depth: 1,
                    });
                self.shared.raw.cmd_copy_image(
                    command,
                    target.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[copy],
                );
            });
        }
        let copy = encoder.finish();
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("sample copied SDR image"),
                });
        encoder.transition_resources(
            std::iter::empty(),
            std::iter::once(wgpu::TextureTransition {
                texture: &*destination,
                selector: None,
                state: wgpu::TextureUses::RESOURCE,
            }),
        );
        let after = encoder.finish();
        let _guard = self.shared.queue_lock.lock();
        let _sync = if self.shared.asynchronous {
            let native = unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }.expect("Vulkan queue");
            native.add_wait_semaphore(
                semaphore,
                Some(signal_value),
                vk::PipelineStageFlags::TOP_OF_PIPE,
            );
            native.add_signal_semaphore(semaphore, Some(signal_value + 1));
            Some(SubmissionSync { queue, semaphore })
        } else {
            None
        };
        // Keep the one-shot wait/signal and submission in the same queue gate.
        let submission = queue.submit([before, copy, after]);
        self.completion.track(submission, target.token as usize);
        Some(resized)
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.shared.pool.lock().stopping = true;
        // Never hold queue/pool locks across mpv teardown: it joins the VO thread.
        unsafe { (self.terminate)(self.handle) };
        self.completion.shutdown();
        let _ = self
            .context
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }
}

#[cfg(test)]
mod tests;
