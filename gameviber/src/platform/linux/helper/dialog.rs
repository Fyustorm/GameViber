//! Brings the polkit password dialog in front of the GUI window.
//!
//! The authentication agent opens its dialog from its own process: without
//! help, the compositor's focus-stealing prevention may keep it behind our
//! window. KDE's agent accepts, per polkit action, an XDG activation token
//! (Wayland) or a parent window id (X11) over D-Bus; we hand it one right
//! before starting pkexec. Other agents (GNOME Shell) already show a modal
//! dialog on top and do not need this; every failure here is harmless.

use std::ffi::c_void;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use anyhow::Context;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{self, XdgActivationTokenV1};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

const KDE_AGENT: &str = "org.kde.polkit-kde-authentication-agent-1";
const KDE_AGENT_PATH: &str = "/org/kde/Polkit1AuthAgent";
const KDE_AGENT_INTERFACE: &str = "org.kde.Polkit1AuthAgent";

#[derive(Clone, Copy)]
enum Window {
    Wayland { display: *mut c_void, surface: *mut c_void },
    X11(u64),
}

// The pointers are only handed to libwayland, which is thread-safe, and are
// cleared (`forget_window`) before the window is destroyed.
unsafe impl Send for Window {}

#[derive(Default)]
struct Gui {
    /// A GUI is starting or running (false when headless).
    expected: bool,
    window: Option<Window>,
    focused: bool,
}

static GUI: Mutex<Gui> = Mutex::new(Gui { expected: false, window: None, focused: false });
static GUI_CHANGED: Condvar = Condvar::new();
/// How long a password request waits for the GUI window to show up and get the focus.
const WINDOW_WAIT: Duration = Duration::from_secs(3);

fn update(f: impl FnOnce(&mut Gui)) {
    f(&mut GUI.lock().unwrap());
    GUI_CHANGED.notify_all();
}

/// Announces that a GUI window is coming: password requests made before it
/// shows up wait for it, otherwise the window would open over the dialog.
pub fn expect_window() {
    update(|gui| gui.expected = true);
}

/// Remembers the GUI window the password dialog should come in front of.
pub fn set_window(window: &(impl HasWindowHandle + HasDisplayHandle)) {
    let (Ok(handle), Ok(display)) = (window.window_handle(), window.display_handle()) else { return };
    let window = match (handle.as_raw(), display.as_raw()) {
        (RawWindowHandle::Wayland(w), RawDisplayHandle::Wayland(d)) => {
            Window::Wayland { display: d.display.as_ptr(), surface: w.surface.as_ptr() }
        }
        (RawWindowHandle::Xlib(w), _) => Window::X11(w.window),
        (RawWindowHandle::Xcb(w), _) => Window::X11(w.window.get().into()),
        _ => return,
    };
    update(|gui| gui.window = Some(window));
}

/// Called every frame: activation tokens are only granted to the focused window.
pub fn set_focused(focused: bool) {
    let mut gui = GUI.lock().unwrap();
    if gui.focused != focused {
        gui.focused = focused;
        GUI_CHANGED.notify_all();
    }
}

/// Must be called before the window goes away.
pub fn forget_window() {
    update(|gui| *gui = Gui::default());
}

/// Lets the next pkexec password dialog open in front of the GUI window.
/// Blocks (briefly) until the window has the focus.
pub fn prepare() {
    let gui = GUI.lock().unwrap();
    let ready = |gui: &mut Gui| !gui.expected || (gui.window.is_some() && gui.focused);
    // The lock is held until the token is handed over, so `forget_window`
    // cannot let the window be destroyed while its surface is in use.
    let (gui, _) = GUI_CHANGED.wait_timeout_while(gui, WINDOW_WAIT, |gui| !ready(gui)).unwrap();
    let Some(window) = gui.window else { return };
    if let Err(e) = tell_kde_agent(window) {
        log::debug!("cannot raise the password dialog: {e:#}");
    }
}

fn tell_kde_agent(window: Window) -> anyhow::Result<()> {
    let bus = zbus::blocking::Connection::session()?;
    let reply = match window {
        Window::Wayland { display, surface } => {
            let token = activation_token(display, surface)?;
            let body = (super::polkit_action(), token.as_str());
            bus.call_method(Some(KDE_AGENT), KDE_AGENT_PATH, Some(KDE_AGENT_INTERFACE), "setActivationTokenForAction", &body)
        }
        Window::X11(id) => {
            let body = (super::polkit_action(), id);
            bus.call_method(Some(KDE_AGENT), KDE_AGENT_PATH, Some(KDE_AGENT_INTERFACE), "setWIdForAction", &body)
        }
    };
    reply.context("KDE polkit agent")?;
    Ok(())
}

/// Asks the compositor for an XDG activation token bound to our surface.
/// Shares the GUI's Wayland connection, on a private event queue.
fn activation_token(display: *mut c_void, surface: *mut c_void) -> anyhow::Result<String> {
    let backend = unsafe { Backend::from_foreign_display(display.cast()) };
    let connection = Connection::from_backend(backend);
    let (globals, mut queue) = registry_queue_init::<TokenState>(&connection)?;
    let qh = queue.handle();
    let activation: XdgActivationV1 = globals.bind(&qh, 1..=1, ()).context("no xdg_activation_v1")?;
    let surface = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }?;
    let surface = WlSurface::from_id(&connection, surface)?;
    let request = activation.get_activation_token(&qh, ());
    request.set_surface(&surface);
    request.commit();
    let mut state = TokenState::default();
    while state.token.is_none() {
        queue.blocking_dispatch(&mut state)?;
    }
    request.destroy();
    activation.destroy();
    Ok(state.token.unwrap_or_default())
}

#[derive(Default)]
struct TokenState {
    token: Option<String>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for TokenState {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgActivationV1, ()> for TokenState {
    fn event(_: &mut Self, _: &XdgActivationV1, _: <XdgActivationV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgActivationTokenV1, ()> for TokenState {
    fn event(
        state: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}
