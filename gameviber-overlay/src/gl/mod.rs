//! OpenGL games: when the library is preloaded (`LD_PRELOAD`, set by the
//! `gameviber-overlay` launcher), it exports `glXSwapBuffers` and
//! `eglSwapBuffers` to draw the overlay right before each swap. Games that
//! fetch these functions at run time (SDL, most engines) go through
//! `glXGetProcAddress`, `eglGetProcAddress` or `dlsym`, so those are wrapped
//! too and hand out the hooks. Everything else is passed to the real
//! libraries untouched. Like the Vulkan layer, nothing is drawn while
//! GameViber is not running.

mod capture;
mod render;

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};
use std::sync::{Mutex, OnceLock};

use crate::client::Client;
use render::Renderer;

type Ptr = *mut c_void;

const GLX_LIBS: [&CStr; 2] = [c"libGLX.so.0", c"libGL.so.1"];
const EGL_LIBS: [&CStr; 1] = [c"libEGL.so.1"];
const GLX_WIDTH: c_int = 0x801D;
const GLX_HEIGHT: c_int = 0x801E;
const EGL_WIDTH: c_int = 0x3057;
const EGL_HEIGHT: c_int = 0x3056;

#[cfg(target_arch = "x86_64")]
const DLSYM_VERSION: &CStr = c"GLIBC_2.2.5";
#[cfg(target_arch = "x86")]
const DLSYM_VERSION: &CStr = c"GLIBC_2.0";

static CLIENT: Mutex<Option<Client>> = Mutex::new(None);
/// Renderer per GL context; `None` when the context cannot host the overlay.
static RENDERERS: Mutex<Option<HashMap<usize, Option<Renderer>>>> = Mutex::new(None);

// The renderers are only used from the thread where their context is current.
unsafe impl Send for Renderer {}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("DISABLE_GAMEVIBER_OVERLAY").is_none_or(|v| v != "1"))
}

/// libc's `dlsym`: ours replaces it for the whole process.
unsafe fn real_dlsym(handle: Ptr, name: *const c_char) -> Ptr {
    static REAL: OnceLock<usize> = OnceLock::new();
    let f = *REAL.get_or_init(|| {
        [c"GLIBC_2.34", DLSYM_VERSION]
            .iter()
            .map(|v| libc::dlvsym(libc::RTLD_NEXT, c"dlsym".as_ptr(), v.as_ptr()) as usize)
            .find(|&p| p != 0)
            .unwrap_or(0)
    });
    if f == 0 {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr = std::mem::transmute(f);
    f(handle, name)
}

/// The real `name` from the GL libraries, whether the game linked them or
/// opened them privately. Never one of our own hooks.
unsafe fn real(libs: &[&CStr], name: &CStr) -> Ptr {
    let ours = hook(name).unwrap_or(std::ptr::null_mut());
    let p = real_dlsym(libc::RTLD_NEXT, name.as_ptr());
    if !p.is_null() && p != ours {
        return p;
    }
    for lib in libs {
        let handle = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY | libc::RTLD_NOLOAD);
        if handle.is_null() {
            continue;
        }
        let p = real_dlsym(handle, name.as_ptr());
        libc::dlclose(handle);
        if !p.is_null() && p != ours {
            return p;
        }
    }
    std::ptr::null_mut()
}

macro_rules! real_fn {
    ($libs:expr, $name:literal, $ty:ty) => {{
        static CACHE: OnceLock<usize> = OnceLock::new();
        let p = match CACHE.get() {
            Some(p) => *p,
            None => {
                let p = real(&$libs, $name) as usize;
                if p != 0 {
                    let _ = CACHE.set(p);
                }
                p
            }
        };
        (p != 0).then(|| std::mem::transmute::<usize, $ty>(p))
    }};
}

type GlxSwapBuffers = unsafe extern "C" fn(Ptr, c_ulong);
type GlxGetProcAddress = unsafe extern "C" fn(*const c_char) -> Ptr;
type GlxQueryDrawable = unsafe extern "C" fn(Ptr, c_ulong, c_int, *mut u32);
type GlxGetCurrentContext = unsafe extern "C" fn() -> Ptr;
type GlxDestroyContext = unsafe extern "C" fn(Ptr, Ptr);
type EglSwapBuffers = unsafe extern "C" fn(Ptr, Ptr) -> u32;
type EglSwapBuffersWithDamage = unsafe extern "C" fn(Ptr, Ptr, *const c_int, c_int) -> u32;
type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> Ptr;
type EglQuerySurface = unsafe extern "C" fn(Ptr, Ptr, c_int, *mut c_int) -> u32;
type EglGetCurrentContext = unsafe extern "C" fn() -> Ptr;
type EglDestroyContext = unsafe extern "C" fn(Ptr, Ptr) -> u32;

/// Our replacement for `name`, if we hook it.
fn hook(name: &CStr) -> Option<Ptr> {
    let f: *const () = match name.to_bytes() {
        b"glXSwapBuffers" => glXSwapBuffers as GlxSwapBuffers as *const (),
        b"glXGetProcAddress" => glXGetProcAddress as GlxGetProcAddress as *const (),
        b"glXGetProcAddressARB" => glXGetProcAddressARB as GlxGetProcAddress as *const (),
        b"glXDestroyContext" => glXDestroyContext as GlxDestroyContext as *const (),
        b"eglSwapBuffers" => eglSwapBuffers as EglSwapBuffers as *const (),
        b"eglSwapBuffersWithDamageKHR" => egl_swap_with_damage_khr as EglSwapBuffersWithDamage as *const (),
        b"eglSwapBuffersWithDamageEXT" => egl_swap_with_damage_ext as EglSwapBuffersWithDamage as *const (),
        b"eglGetProcAddress" => eglGetProcAddress as EglGetProcAddress as *const (),
        b"eglDestroyContext" => eglDestroyContext as EglDestroyContext as *const (),
        b"dlsym" => dlsym as unsafe extern "C" fn(Ptr, *const c_char) -> Ptr as *const (),
        _ => return None,
    };
    Some(f as Ptr)
}

#[no_mangle]
pub unsafe extern "C" fn dlsym(handle: Ptr, name: *const c_char) -> Ptr {
    if !name.is_null() && enabled() {
        if let Some(f) = hook(CStr::from_ptr(name)) {
            return f;
        }
    }
    real_dlsym(handle, name)
}

#[no_mangle]
pub unsafe extern "C" fn glXGetProcAddress(name: *const c_char) -> Ptr {
    glx_get_proc_address(name, c"glXGetProcAddress")
}

#[no_mangle]
pub unsafe extern "C" fn glXGetProcAddressARB(name: *const c_char) -> Ptr {
    glx_get_proc_address(name, c"glXGetProcAddressARB")
}

unsafe fn glx_get_proc_address(name: *const c_char, which: &CStr) -> Ptr {
    if !name.is_null() && enabled() {
        if let Some(f) = hook(CStr::from_ptr(name)) {
            return f;
        }
    }
    match real(&GLX_LIBS, which) {
        p if p.is_null() => p,
        p => std::mem::transmute::<Ptr, GlxGetProcAddress>(p)(name),
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglGetProcAddress(name: *const c_char) -> Ptr {
    if !name.is_null() && enabled() {
        if let Some(f) = hook(CStr::from_ptr(name)) {
            return f;
        }
    }
    match real_fn!(EGL_LIBS, c"eglGetProcAddress", EglGetProcAddress) {
        Some(f) => f(name),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glXSwapBuffers(dpy: Ptr, drawable: c_ulong) {
    if enabled() {
        guard(|| draw_glx(dpy, drawable));
    }
    if let Some(f) = real_fn!(GLX_LIBS, c"glXSwapBuffers", GlxSwapBuffers) {
        f(dpy, drawable);
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglSwapBuffers(dpy: Ptr, surface: Ptr) -> u32 {
    if enabled() {
        guard(|| draw_egl(dpy, surface));
    }
    match real_fn!(EGL_LIBS, c"eglSwapBuffers", EglSwapBuffers) {
        Some(f) => f(dpy, surface),
        None => 0,
    }
}

unsafe extern "C" fn egl_swap_with_damage_khr(dpy: Ptr, surface: Ptr, rects: *const c_int, n: c_int) -> u32 {
    if enabled() {
        guard(|| draw_egl(dpy, surface));
    }
    let real: Option<EglSwapBuffersWithDamage> =
        real_fn!(EGL_LIBS, c"eglGetProcAddress", EglGetProcAddress).and_then(|get| {
            let p = get(c"eglSwapBuffersWithDamageKHR".as_ptr());
            (!p.is_null() && p != egl_swap_with_damage_khr as *mut c_void).then(|| std::mem::transmute(p))
        });
    match real {
        Some(f) => f(dpy, surface, rects, n),
        None => 0,
    }
}

unsafe extern "C" fn egl_swap_with_damage_ext(dpy: Ptr, surface: Ptr, rects: *const c_int, n: c_int) -> u32 {
    if enabled() {
        guard(|| draw_egl(dpy, surface));
    }
    let real: Option<EglSwapBuffersWithDamage> =
        real_fn!(EGL_LIBS, c"eglGetProcAddress", EglGetProcAddress).and_then(|get| {
            let p = get(c"eglSwapBuffersWithDamageEXT".as_ptr());
            (!p.is_null() && p != egl_swap_with_damage_ext as *mut c_void).then(|| std::mem::transmute(p))
        });
    match real {
        Some(f) => f(dpy, surface, rects, n),
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glXDestroyContext(dpy: Ptr, ctx: Ptr) {
    forget_context(ctx);
    if let Some(f) = real_fn!(GLX_LIBS, c"glXDestroyContext", GlxDestroyContext) {
        f(dpy, ctx);
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglDestroyContext(dpy: Ptr, ctx: Ptr) -> u32 {
    forget_context(ctx);
    match real_fn!(EGL_LIBS, c"eglDestroyContext", EglDestroyContext) {
        Some(f) => f(dpy, ctx),
        None => 0,
    }
}

/// The context may not be current: its GL objects go away with it.
fn forget_context(ctx: Ptr) {
    if let Some(map) = RENDERERS.lock().unwrap().as_mut() {
        if let Some(Some(renderer)) = map.remove(&(ctx as usize)) {
            std::mem::forget(renderer);
        }
    }
}

/// Never let an overlay bug take the game down.
fn guard(f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        crate::log("OpenGL overlay crashed");
    }
}

unsafe fn draw_glx(dpy: Ptr, drawable: c_ulong) {
    let Some(current) = real_fn!(GLX_LIBS, c"glXGetCurrentContext", GlxGetCurrentContext) else { return };
    let ctx = current();
    if ctx.is_null() {
        return;
    }
    let (mut w, mut h) = (0u32, 0u32);
    if let Some(query) = real_fn!(GLX_LIBS, c"glXQueryDrawable", GlxQueryDrawable) {
        query(dpy, drawable, GLX_WIDTH, &mut w);
        query(dpy, drawable, GLX_HEIGHT, &mut h);
    }
    let Some(get) = real_fn!(GLX_LIBS, c"glXGetProcAddressARB", GlxGetProcAddress) else { return };
    draw(ctx as usize, w, h, |name| get(name.as_ptr()) as *const c_void);
}

unsafe fn draw_egl(dpy: Ptr, surface: Ptr) {
    let Some(current) = real_fn!(EGL_LIBS, c"eglGetCurrentContext", EglGetCurrentContext) else { return };
    let ctx = current();
    if ctx.is_null() {
        return;
    }
    let (mut w, mut h) = (0 as c_int, 0 as c_int);
    if let Some(query) = real_fn!(EGL_LIBS, c"eglQuerySurface", EglQuerySurface) {
        query(dpy, surface, EGL_WIDTH, &mut w);
        query(dpy, surface, EGL_HEIGHT, &mut h);
    }
    let Some(get) = real_fn!(EGL_LIBS, c"eglGetProcAddress", EglGetProcAddress) else { return };
    draw(ctx as usize, w.max(0) as u32, h.max(0) as u32, |name| get(name.as_ptr()) as *const c_void);
}

unsafe fn draw(ctx: usize, width: u32, height: u32, load: impl FnMut(&CStr) -> *const c_void) {
    let mut client = CLIENT.lock().unwrap();
    let client = client.get_or_insert_with(|| Client::new("opengl"));
    let state = match client.poll() {
        Some(state) => state.clone(),
        None => return,
    };
    let mut renderers = RENDERERS.lock().unwrap();
    let renderer = renderers.get_or_insert_with(HashMap::new).entry(ctx).or_insert_with(|| match Renderer::new(load) {
        Ok(r) => {
            crate::log(&format!("OpenGL overlay ready ({}, {width}x{height})", r.version()));
            Some(r)
        }
        Err(e) => {
            crate::log(&format!("OpenGL overlay unavailable in this context: {e}"));
            None
        }
    });
    if let Some(renderer) = renderer {
        renderer.draw(&state, width, height, client);
    }
}
