// All native pointers and callbacks are confined to this worker-owned module.
#![allow(unsafe_code)]

use super::color::{BASE_FILTER, Plan};
use super::{Frame, HEIGHT, Request, Shared, WIDTH};
use iced_core::image;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    sync::Arc,
    time::{Duration, Instant},
};

const NATIVE_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct Property {
    _name: *const c_char,
    format: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct Param {
    kind: c_int,
    data: *mut c_void,
}
impl Param {
    fn new(kind: c_int, data: *mut c_void) -> Self {
        Self { kind, data }
    }
    fn end() -> Self {
        Self::new(0, std::ptr::null_mut())
    }
}

type Callback = Option<unsafe extern "C" fn(*mut c_void)>;
struct Api {
    create: unsafe extern "C" fn() -> *mut c_void,
    terminate: unsafe extern "C" fn(*mut c_void),
    option: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    command: unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> c_int,
    property: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    event: unsafe extern "C" fn(*mut c_void, f64) -> *mut Event,
    wakeup: unsafe extern "C" fn(*mut c_void, Callback, *mut c_void),
    error: unsafe extern "C" fn(c_int) -> *const c_char,
    create_render: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut Param) -> c_int,
    free_render: unsafe extern "C" fn(*mut c_void),
    update_callback: unsafe extern "C" fn(*mut c_void, Callback, *mut c_void),
    update: unsafe extern "C" fn(*mut c_void) -> u64,
    render: unsafe extern "C" fn(*mut c_void, *mut Param) -> c_int,
    _library: libloading::Library,
}
impl Api {
    fn load(path: &str) -> Result<Self, String> {
        // All symbols have the signatures declared by client.h and render.h.
        // The library outlives the core, context, and their callbacks.
        unsafe {
            let library = libloading::Library::new(path).map_err(|error| error.to_string())?;
            macro_rules! symbol {
                ($name:literal) => {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|error| error.to_string())?
                };
            }
            Ok(Self {
                create: symbol!("mpv_create"),
                terminate: symbol!("mpv_terminate_destroy"),
                option: symbol!("mpv_set_option_string"),
                initialize: symbol!("mpv_initialize"),
                command: symbol!("mpv_command_async"),
                property: symbol!("mpv_get_property_async"),
                event: symbol!("mpv_wait_event"),
                wakeup: symbol!("mpv_set_wakeup_callback"),
                error: symbol!("mpv_error_string"),
                create_render: symbol!("mpv_render_context_create"),
                free_render: symbol!("mpv_render_context_free"),
                update_callback: symbol!("mpv_render_context_set_update_callback"),
                update: symbol!("mpv_render_context_update"),
                render: symbol!("mpv_render_context_render"),
                _library: library,
            })
        }
    }
    fn check(&self, status: c_int) -> Result<(), String> {
        if status < 0 {
            Err(unsafe { CStr::from_ptr((self.error)(status)) }
                .to_string_lossy()
                .into_owned())
        } else {
            Ok(())
        }
    }
}

#[repr(align(64))]
struct Pixels([u32; (WIDTH * HEIGHT) as usize]);

pub(super) struct Decoder {
    api: Api,
    core: *mut c_void,
    render: *mut c_void,
    pixels: Box<Pixels>,
    signal: Arc<Shared>,
    loaded: bool,
    ready: bool,
    request: Option<Request>,
    acknowledged: bool,
    started: bool,
    snapshot: bool,
    position: Option<f64>,
    gamma: Option<String>,
    matrix: Option<String>,
    primaries: Option<String>,
    source_matrix: Option<String>,
    profile: Option<Option<i64>>,
    color_plan: Option<Plan>,
    pub(super) reconfigure: Option<Plan>,
    seekable: bool,
    deadline: Option<Instant>,
    #[cfg(test)]
    lose_completion: bool,
}

// Native callbacks only wake the worker. No mailbox lock, native call, render,
// allocation, or UI work is performed on mpv's callback threads.
unsafe extern "C" fn wake(opaque: *mut c_void) {
    let shared = unsafe { &*opaque.cast::<Shared>() };
    shared.wake();
}

impl Decoder {
    pub(super) fn new(
        library: &str,
        source: &str,
        signal: Arc<Shared>,
        color_plan: Option<Plan>,
    ) -> Result<Self, String> {
        let api = Api::load(library)?;
        let core = unsafe { (api.create)() };
        if core.is_null() {
            return Err("preview mpv_create failed".into());
        }
        let mut decoder = Self {
            api,
            core,
            render: std::ptr::null_mut(),
            pixels: Box::new(Pixels([0; (WIDTH * HEIGHT) as usize])),
            signal,
            loaded: false,
            ready: false,
            request: None,
            acknowledged: false,
            started: false,
            snapshot: false,
            position: None,
            gamma: None,
            matrix: None,
            primaries: None,
            source_matrix: None,
            profile: None,
            color_plan,
            reconfigure: None,
            seekable: false,
            deadline: Some(Instant::now() + NATIVE_OPERATION_TIMEOUT),
            #[cfg(test)]
            lose_completion: false,
        };
        // Synchronous configuration is legal here: no render context exists yet.
        for (name, value) in [
            ("config", "no"),
            ("vo", "libmpv"),
            ("pause", "yes"),
            ("idle", "yes"),
            ("keep-open", "always"),
            ("hwdec", "no"),
            ("aid", "no"),
            ("sid", "no"),
            ("osd-level", "0"),
            ("load-scripts", "no"),
            ("terminal", "no"),
            ("video-timing-offset", "0"),
            ("vd-lavc-threads", "2"),
            ("cache", "no"),
            ("demuxer-seekable-cache", "no"),
            ("demuxer-max-bytes", "8MiB"),
            ("demuxer-max-back-bytes", "0"),
        ] {
            let name = CString::new(name).expect("option name");
            let value = CString::new(value).expect("option value");
            decoder
                .api
                .check(unsafe { (decoder.api.option)(core, name.as_ptr(), value.as_ptr()) })?;
        }
        let filter = CString::new(color_plan.map_or_else(|| BASE_FILTER.to_owned(), Plan::filter))
            .expect("static filter syntax");
        decoder
            .api
            .check(unsafe { (decoder.api.option)(core, c"vf".as_ptr(), filter.as_ptr()) })?;
        decoder
            .api
            .check(unsafe { (decoder.api.initialize)(core) })?;
        let opaque = Arc::as_ptr(&decoder.signal).cast_mut().cast();
        unsafe { (decoder.api.wakeup)(core, Some(wake), opaque) };
        let mut params = [
            Param::new(1, c"sw".as_ptr().cast_mut().cast()),
            Param::end(),
        ];
        decoder.api.check(unsafe {
            (decoder.api.create_render)(&mut decoder.render, core, params.as_mut_ptr())
        })?;
        unsafe { (decoder.api.update_callback)(decoder.render, Some(wake), opaque) };
        let source = CString::new(source).map_err(|_| "preview source contains a NUL byte")?;
        decoder.command(1, &[c"loadfile", &source])?;
        Ok(decoder)
    }

    fn command(&self, id: u64, args: &[&CStr]) -> Result<(), String> {
        let mut pointers = [std::ptr::null(); 4];
        for (slot, arg) in pointers.iter_mut().zip(args) {
            *slot = arg.as_ptr();
        }
        self.api
            .check(unsafe { (self.api.command)(self.core, id, pointers.as_ptr()) })
    }

    pub(super) fn idle(&self) -> bool {
        self.ready && self.seekable && self.request.is_none() && !self.snapshot
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn seek(&mut self, request: Request) -> Result<(), String> {
        assert!(self.idle());
        self.request = Some(request);
        self.deadline = Some(Instant::now() + NATIVE_OPERATION_TIMEOUT);
        self.acknowledged = false;
        self.started = false;
        let target = CString::new(request.seconds().to_string()).expect("finite target");
        self.command(2, &[c"seek", &target, c"absolute+exact"])
    }

    fn snapshot(&mut self) -> Result<(), String> {
        self.snapshot = true;
        self.position = None;
        self.gamma = None;
        self.matrix = None;
        self.primaries = None;
        self.source_matrix = None;
        self.profile = None;
        for (id, name, format) in [
            (10, c"time-pos", 5),
            (11, c"video-out-params/gamma", 1),
            (12, c"video-out-params/colormatrix", 1),
            (14, c"video-out-params/primaries", 1),
            (15, c"video-params/colormatrix", 1),
            (16, c"current-tracks/video/dolby-vision-profile", 4),
        ] {
            self.api
                .check(unsafe { (self.api.property)(self.core, id, name.as_ptr(), format) })?;
        }
        Ok(())
    }

    fn render(&mut self) -> Result<(), String> {
        let mut size = [WIDTH as c_int, HEIGHT as c_int];
        let mut stride = (WIDTH * 4) as usize;
        let mut block = 0_i32;
        let mut params = [
            Param::new(17, size.as_mut_ptr().cast()),
            Param::new(18, c"rgb0".as_ptr().cast_mut().cast()),
            Param::new(19, (&mut stride as *mut usize).cast()),
            Param::new(20, self.pixels.0.as_mut_ptr().cast()),
            Param::new(12, (&mut block as *mut i32).cast()),
            Param::end(),
        ];
        self.api
            .check(unsafe { (self.api.render)(self.render, params.as_mut_ptr()) })
    }

    pub(super) fn drain(&mut self) -> Result<Option<(Request, Frame)>, String> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(
                "Thumbnail unavailable: native operation timed out; reload media to retry".into(),
            );
        }
        // Rendering must continue while waiting for PLAYBACK_RESTART, otherwise
        // the core can wait for a frame that this thread never consumes.
        if unsafe { (self.api.update)(self.render) } & 1 != 0 {
            self.render()?;
        }
        let mut drained = false;
        for _ in 0..256 {
            let event = unsafe { &*(self.api.event)(self.core, 0.0) };
            match event.id {
                0 => {
                    drained = true;
                    break;
                }
                1 => return Err("preview player shut down".into()),
                7 => return Err("preview media ended or could not be decoded".into()),
                8 => {
                    self.loaded = true;
                    self.api.check(unsafe {
                        (self.api.property)(self.core, 13, c"seekable".as_ptr(), 3)
                    })?;
                }
                5 => {
                    self.api.check(event.error)?;
                    if event.userdata == 2 {
                        self.acknowledged = true;
                        #[cfg(test)]
                        if let Some(gate) = self.signal.ack_gate.lock().take() {
                            let _ = gate.reached.send(self.request.expect("submitted seek"));
                            if matches!(
                                gate.proceed.recv_timeout(std::time::Duration::from_secs(2)),
                                Ok(super::tests::AckAction::SuppressCompletion),
                            ) {
                                self.lose_completion = true;
                                self.deadline =
                                    Some(Instant::now() + std::time::Duration::from_millis(200));
                            }
                        }
                    }
                }
                #[cfg(test)]
                20 | 21 if self.lose_completion => {}
                20 if self.request.is_some() && self.acknowledged => self.started = true,
                21 if !self.snapshot
                    && ((self.request.is_some() && self.started)
                        || (!self.ready && self.loaded)) =>
                {
                    self.snapshot()?;
                }
                3 if event.userdata == 13 => {
                    self.api.check(event.error)?;
                    if event.data.is_null() {
                        return Err("Thumbnail unavailable: seek capability missing".into());
                    }
                    let property = unsafe { &*event.data.cast::<Property>() };
                    self.seekable = property.format == 3
                        && !property.data.is_null()
                        && unsafe { *property.data.cast::<c_int>() } != 0;
                    if !self.seekable {
                        return Err("Thumbnail unavailable: source is not seekable".into());
                    }
                }
                3 if self.snapshot && event.userdata == 16 => {
                    // Ordinary SDR/HDR tracks have no Dolby Vision property.
                    let profile = if event.error >= 0 && !event.data.is_null() {
                        let property = unsafe { &*event.data.cast::<Property>() };
                        (property.format == 4 && !property.data.is_null())
                            .then(|| unsafe { *property.data.cast::<i64>() })
                    } else {
                        None
                    };
                    self.profile = Some(profile);
                }
                3 if self.snapshot => {
                    self.api.check(event.error)?;
                    if event.data.is_null() {
                        return Err("preview property payload missing".into());
                    }
                    let property = unsafe { &*event.data.cast::<Property>() };
                    if property.data.is_null() {
                        return Err("preview property unavailable".into());
                    }
                    match (event.userdata, property.format) {
                        (10, 5) => {
                            let value = unsafe { *property.data.cast::<f64>() };
                            if !value.is_finite() {
                                return Err("preview timestamp invalid".into());
                            }
                            self.position = Some(value);
                        }
                        (id @ (11 | 12 | 14 | 15), 1) => {
                            let pointer = unsafe { *property.data.cast::<*const c_char>() };
                            if pointer.is_null() {
                                return Err("preview color metadata unavailable".into());
                            }
                            let value = unsafe { CStr::from_ptr(pointer) }
                                .to_string_lossy()
                                .into_owned();
                            match id {
                                11 => self.gamma = Some(value),
                                12 => self.matrix = Some(value),
                                14 => self.primaries = Some(value),
                                15 => self.source_matrix = Some(value),
                                _ => unreachable!(),
                            }
                        }
                        _ => return Err("unexpected preview property format".into()),
                    }
                }
                24 => return Err("preview event queue overflow".into()),
                _ => {}
            }
            if unsafe { (self.api.update)(self.render) } & 1 != 0 {
                self.render()?;
            }
        }
        if !drained {
            self.signal.wake();
        }
        if self.snapshot
            && self.seekable
            && let (
                Some(position),
                Some(gamma),
                Some(matrix),
                Some(primaries),
                Some(source_matrix),
                Some(profile),
            ) = (
                self.position,
                &self.gamma,
                &self.matrix,
                &self.primaries,
                &self.source_matrix,
                self.profile,
            )
        {
            let Some(plan) = self.color_plan else {
                // Probe first, then reopen with a fixed filter graph. This keeps
                // reconfiguration restart events out of the seek state machine.
                self.reconfigure = Some(Plan::classify(
                    gamma,
                    matrix,
                    primaries,
                    source_matrix,
                    profile,
                )?);
                self.snapshot = false;
                return Ok(None);
            };
            if gamma != "srgb" || matrix != "rgb" || primaries != "bt.709" {
                return Err(format!(
                    "Thumbnail unavailable: color conversion did not produce sRGB/BT.709 RGB ({gamma}, {matrix}, {primaries})"
                ));
            }
            self.snapshot = false;
            self.ready = true;
            if let Some(request) = self.request.take() {
                self.deadline = None;
                self.render()?;
                let mut rgba = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
                for pixel in &self.pixels.0 {
                    let [r, g, b, _] = pixel.to_ne_bytes();
                    // rgb0's last byte is unspecified, not alpha.
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                return Ok(Some((
                    request,
                    Frame {
                        requested: request.seconds(),
                        actual: position,
                        color: plan.label(),
                        image: image::Handle::from_rgba(WIDTH, HEIGHT, rgba),
                    },
                )));
            }
        }
        if self.idle() {
            self.deadline = None;
        }
        Ok(None)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // The render context must die before the core. All callbacks retain a
        // live signal until terminate_destroy has joined mpv's threads.
        unsafe {
            if !self.render.is_null() {
                (self.api.update_callback)(self.render, None, std::ptr::null_mut());
                (self.api.free_render)(self.render);
            }
            (self.api.wakeup)(self.core, None, std::ptr::null_mut());
            (self.api.terminate)(self.core);
        }
    }
}
