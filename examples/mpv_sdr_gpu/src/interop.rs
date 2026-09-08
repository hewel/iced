use crate::{Options, gpu::DeviceContext};
use ash::vk;
use iced_wgpu::wgpu;
use parking_lot::{Mutex, RawMutex, lock_api::RawMutex as _};
use std::{
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_int, c_void},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug, Clone, Copy)]
pub enum Command {
    TogglePause,
    SeekAbsolute(f64),
    SeekRelative(f64),
    SetVolume(f64),
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
    EndFile { error: Option<String> },
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
const MPV_EVENT_COMMAND_REPLY: c_int = 5;
const MPV_EVENT_START_FILE: c_int = 6;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_FILE_LOADED: c_int = 8;
const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;
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

// Exact repr(C) translation of mpv/include/mpv/gpu_next.h version 1.
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
}

pub struct QueueLock(RawMutex);
impl QueueLock {
    fn new() -> Self {
        Self(RawMutex::INIT)
    }
    pub fn lock(&self) -> QueueGuard<'_> {
        self.0.lock();
        QueueGuard(self)
    }
}
pub struct QueueGuard<'a>(&'a QueueLock);
impl Drop for QueueGuard<'_> {
    fn drop(&mut self) {
        unsafe { self.0.0.unlock() }
    }
}

struct Slot {
    target: Target,
    memory: vk::DeviceMemory,
    busy: bool,
}
struct Pool {
    slots: Vec<Slot>,
    ready: VecDeque<usize>,
    stopping: bool,
    size: (u32, u32),
}
struct Shared {
    raw: ash::Device,
    memory: vk::PhysicalDeviceMemoryProperties,
    family: u32,
    queue_lock: Arc<QueueLock>,
    pool: Mutex<Pool>,
    exhausted: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
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
                busy: false,
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
    let Some(mut pool) = shared.pool.try_lock() else {
        shared.exhausted.store(true, Ordering::Release);
        return 0;
    };
    if pool.stopping {
        return 0;
    }
    let size = pool.size;
    let Some(slot) = pool
        .slots
        .iter_mut()
        .find(|s| !s.busy && (s.target.width as u32, s.target.height as u32) == size)
    else {
        shared.exhausted.store(true, Ordering::Release);
        return 0;
    };
    slot.busy = true;
    unsafe { out.write(slot.target) };
    1
}
unsafe extern "C" fn release(opaque: *mut c_void, target: *const Target, status: c_int) {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    let target = unsafe { *target };
    {
        let mut pool = shared.pool.lock();
        let index = target.token as usize;
        let slot = &mut pool.slots[index];
        if status == 0 {
            slot.target.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
            pool.ready.push_back(index);
        } else {
            slot.target.layout = vk::ImageLayout::UNDEFINED;
            slot.busy = false;
        }
    }
    (shared.wake)();
}

pub struct Host {
    handle: *mut c_void,
    terminate: unsafe extern "C" fn(*mut c_void),
    request_redraw: unsafe extern "C" fn(*mut c_void) -> c_int,
    command_async: unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    _library: libloading::Library,
    _descriptor: Box<Descriptor>,
    _extensions: Vec<*const c_char>,
    shared: Arc<Shared>,
    device: wgpu::Device,
}
impl Host {
    pub fn new(
        context: &DeviceContext,
        options: &Options,
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
                family: native.queue_family_index(),
                memory,
                queue_lock: Arc::new(QueueLock::new()),
                pool: Mutex::new(Pool {
                    slots: Vec::new(),
                    ready: VecDeque::new(),
                    stopping: false,
                    size: (options.width, options.height),
                }),
                exhausted: AtomicBool::new(false),
                wake: Box::new(wake),
            });
            for index in 0..3 {
                let slot = shared.allocate(options.width, options.height, index)?;
                shared.pool.lock().slots.push(slot);
            }
            let extensions: Vec<_> = native
                .enabled_device_extensions()
                .iter()
                .map(|e| e.as_ptr())
                .collect();
            let descriptor = Box::new(Descriptor {
                version: 1,
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
                error_string,
                _library: library,
                _descriptor: descriptor,
                _extensions: extensions,
                shared,
                device: context.device.clone(),
            };
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

    fn error(&self, status: c_int) -> String {
        // mpv_error_string returns static storage, never owned by the caller.
        unsafe { CStr::from_ptr((self.error_string)(status)) }
            .to_string_lossy()
            .into_owned()
    }

    /// Application-thread only, outside the queue lock and every VO callback.
    /// Success means queued; execution failures arrive through poll_events.
    pub fn execute(&self, command: Command) -> Result<(), String> {
        let (args, id);
        let number;
        match command {
            Command::TogglePause => {
                args = [c"cycle", c"pause", c""];
                id = 1;
            }
            Command::SeekAbsolute(value)
            | Command::SeekRelative(value)
            | Command::SetVolume(value) => {
                if !value.is_finite() {
                    return Err("mpv command requires a finite value".into());
                }
                if matches!(command, Command::SeekAbsolute(_) | Command::SetVolume(_))
                    && value < 0.0
                {
                    return Err("mpv absolute position and volume must be nonnegative".into());
                }
                number = CString::new(value.to_string()).map_err(|error| error.to_string())?;
                (args, id) = match command {
                    Command::SeekAbsolute(_) => {
                        ([c"seek", number.as_c_str(), c"absolute+exact"], 2)
                    }
                    Command::SeekRelative(_) => {
                        ([c"seek", number.as_c_str(), c"relative+exact"], 3)
                    }
                    Command::SetVolume(_) => ([c"set", c"volume", number.as_c_str()], 4),
                    Command::TogglePause => unreachable!(),
                };
            }
        }
        let pointers = [
            args[0].as_ptr(),
            args[1].as_ptr(),
            if id == 1 {
                std::ptr::null()
            } else {
                args[2].as_ptr()
            },
            std::ptr::null(),
        ];
        // mpv copies command arguments before returning.
        let status = unsafe { (self.command_async)(self.handle, id, pointers.as_ptr()) };
        if status < 0 {
            Err(format!("mpv command failed: {}", self.error(status)))
        } else {
            Ok(())
        }
    }

    /// Application-thread only, outside the queue lock and every VO callback.
    /// All payloads are copied before the next wait_event invalidates them.
    pub fn poll_events(&self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        // Yield to the application during bursts, scheduling another turn even
        // if every event in this batch was an ignored future event type.
        for _ in 0..256 {
            let event = unsafe { &*(self.wait_event)(self.handle, 0.0) };
            match event.event_id {
                MPV_EVENT_NONE => return events,
                MPV_EVENT_SHUTDOWN => events.push(PlayerEvent::Shutdown),
                MPV_EVENT_START_FILE => events.push(PlayerEvent::StartFile),
                MPV_EVENT_FILE_LOADED => events.push(PlayerEvent::FileLoaded),
                MPV_EVENT_END_FILE if !event.data.is_null() => {
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
                    !slot.busy && (slot.target.width as u32, slot.target.height as u32) == pool.size
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
                let size = pool.size;
                let slot = &mut pool.slots[index];
                if slot.busy || (slot.target.width as u32, slot.target.height as u32) == size {
                    continue;
                }
                // Reserve only GPU-idle slots. Allocation never holds the pool
                // lock, so VO callbacks cannot block behind a Vulkan allocation.
                slot.busy = true;
                size
            };
            let replacement = match self.shared.allocate(size.0, size.1, index as u64) {
                Ok(slot) => slot,
                Err(error) => {
                    self.shared.pool.lock().slots[index].busy = false;
                    return Err(error);
                }
            };
            let old = std::mem::replace(&mut self.shared.pool.lock().slots[index], replacement);
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

    // Caller holds the shared queue lock. No mpv API is called here. Ready means
    // flip_page has authorized display AND producer GPU writes have completed.
    pub fn copy_ready(&self, queue: &wgpu::Queue, destination: &mut wgpu::Texture) -> Option<bool> {
        let target = {
            let mut pool = self.shared.pool.lock();
            let index = pool.ready.pop_front()?;
            pool.slots[index].target
        };
        let resized = destination.width() != target.width as u32
            || destination.height() != target.height as u32;
        if resized {
            *destination = Self::private_texture(
                &self.device,
                queue,
                target.width as u32,
                target.height as u32,
            );
        }
        let mut encoder = self
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
        // wgpu 29 forbids mixing its encoder API with raw HAL recording.
        // Keep tracked transitions in separate command buffers around the copy.
        let mut encoder = self
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
        let mut encoder = self
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
        queue.submit([before, copy, encoder.finish()]);
        let shared = self.shared.clone();
        queue.on_submitted_work_done(move || {
            {
                let mut pool = shared.pool.lock();
                let slot = &mut pool.slots[target.token as usize];
                slot.target.layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
                slot.busy = false;
            }
            (shared.wake)();
        });
        Some(resized)
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.shared.pool.lock().stopping = true;
        // Never hold queue/pool locks across mpv teardown: it joins the VO thread.
        unsafe { (self.terminate)(self.handle) };
        let _guard = self.shared.queue_lock.lock();
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}
