//! Keyboard shortcuts for the gamepad combos' actions (stop, mark a moment,
//! capture the screen), through the desktop's global shortcuts portal
//! (`org.freedesktop.portal.GlobalShortcuts`): the desktop owns the keys, so
//! the game never sees them, and no input device permission is needed. The
//! desktop asks the player to confirm the keys the first time, and lets them
//! change them later.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
/// The id the portal knows GameViber by, which needs a desktop entry of that name
/// (else the desktop files the shortcuts under the terminal GameViber started from).
pub const APP_ID: &str = "io.github.gameviber.GameViber";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Panic,
    Mark,
    Capture,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Panic, Action::Mark, Action::Capture];

    fn id(self) -> &'static str {
        match self {
            Action::Panic => "panic",
            Action::Mark => "mark",
            Action::Capture => "capture",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Action::Panic => "GameViber: stop every toy",
            Action::Mark => "GameViber: mark a moment that felt wrong",
            Action::Capture => "GameViber: capture the game's screen",
        }
    }

    /// Keys suggested to the desktop (it may pick others).
    fn preferred(self) -> &'static str {
        match self {
            Action::Panic => "CTRL+ALT+X",
            Action::Mark => "CTRL+ALT+M",
            Action::Capture => "CTRL+ALT+C",
        }
    }
}

/// The shortcuts as the GUI shows them.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    Connecting,
    /// Bound: the keys of each action, as the desktop describes them ("" if none).
    Ready(Vec<(Action, String)>),
    Failed(String),
}

pub struct Shortcuts {
    actions: Receiver<Action>,
    status: Arc<Mutex<Status>>,
    /// The connection and session, once bound, to open the desktop's settings.
    session: Arc<Mutex<Option<(Connection, OwnedObjectPath)>>>,
}

impl Shortcuts {
    pub fn start() -> Self {
        let (tx, actions) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::Connecting));
        let session = Arc::new(Mutex::new(None));
        let (st, se) = (status.clone(), session.clone());
        std::thread::Builder::new()
            .name("shortcuts".into())
            .spawn(move || {
                if let Err(e) = run(&tx, &st, &se) {
                    log::warn!("keyboard shortcuts unavailable: {e:#}");
                    *st.lock().unwrap() = Status::Failed(format!("{e:#}"));
                }
            })
            .expect("spawn shortcuts thread");
        Self { actions, status, session }
    }

    /// Actions whose keys were pressed since the last call.
    pub fn poll(&self) -> Vec<Action> {
        self.actions.try_iter().collect()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Opens the desktop's settings of these shortcuts, to change their keys.
    pub fn configure(&self) {
        let session = self.session.lock().unwrap().clone();
        let Some((conn, handle)) = session else { return };
        std::thread::spawn(move || {
            let options: HashMap<&str, Value> = HashMap::new();
            if let Err(e) = conn.call_method(Some(PORTAL), PORTAL_PATH, Some(SHORTCUTS), "ConfigureShortcuts", &(&handle, "", options)) {
                log::warn!("cannot open the shortcut settings: {e}");
            }
        });
    }
}

/// A portal request: calls `method` (whose options carry `token` as their
/// handle token), and waits for its response.
fn request<B>(conn: &Connection, method: &str, token: &str, body: &B) -> anyhow::Result<HashMap<String, OwnedValue>>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let sender = conn.unique_name().context("no bus name")?.trim_start_matches(':').replace('.', "_");
    let path = format!("{PORTAL_PATH}/request/{sender}/{token}");
    let request = Proxy::new(conn, PORTAL, path, "org.freedesktop.portal.Request")?;
    // Listening before calling, not to miss a quick response.
    let mut responses = request.receive_signal("Response")?;
    conn.call_method(Some(PORTAL), PORTAL_PATH, Some(SHORTCUTS), method, body).with_context(|| method.to_owned())?;
    let message = responses.next().context("no response")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
    match code {
        0 => Ok(results),
        1 => bail!("{method}: refused"),
        _ => bail!("{method}: failed"),
    }
}

type Bound = Vec<(String, HashMap<String, OwnedValue>)>;

/// The keys of each action, from the shortcuts the portal sent.
fn triggers(bound: &Bound) -> Vec<(Action, String)> {
    Action::ALL
        .iter()
        .map(|action| {
            let keys = bound
                .iter()
                .find(|(id, _)| id == action.id())
                .and_then(|(_, props)| props.get("trigger_description"))
                .and_then(|v| v.downcast_ref::<String>().ok())
                .unwrap_or_default();
            (*action, keys)
        })
        .collect()
}

/// Tells the portal who we are, before any other request: writes our desktop
/// entry if needed, and waits a little for the portal to see it.
fn register(conn: &Connection) {
    if let Err(e) = desktop_entry() {
        log::warn!("cannot write the desktop entry: {e:#}");
    }
    for attempt in 0..6 {
        let options: HashMap<&str, Value> = HashMap::new();
        match conn.call_method(Some(PORTAL), PORTAL_PATH, Some("org.freedesktop.host.portal.Registry"), "Register", &(APP_ID, options)) {
            Ok(_) => return,
            // Older portals lack the registry.
            Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == "org.freedesktop.DBus.Error.UnknownInterface" => return,
            Err(e) if attempt == 5 => log::warn!("the desktop does not know GameViber: {e}"),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
}

/// `~/.local/share/applications/<APP_ID>.desktop`, launching this executable.
fn desktop_entry() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let dir = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .context("no home")?
        .join("applications");
    let path = dir.join(format!("{APP_ID}.desktop"));
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=GameViber\nComment=Game rumble, sound and image to toys\nExec={}\nIcon=input-gaming\nCategories=Game;Utility;\n",
        exe.display()
    );
    if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&path, entry)?;
        log::info!("desktop entry written: {}", path.display());
    }
    Ok(())
}

fn run(tx: &Sender<Action>, status: &Arc<Mutex<Status>>, session: &Mutex<Option<(Connection, OwnedObjectPath)>>) -> anyhow::Result<()> {
    let conn = Connection::session()?;
    register(&conn);
    let options = HashMap::from([("handle_token", Value::from("gameviber_session")), ("session_handle_token", Value::from("gameviber"))]);
    let results = request(&conn, "CreateSession", "gameviber_session", &(options,))?;
    let handle: String = results.get("session_handle").and_then(|v| v.downcast_ref::<String>().ok()).context("no session")?;
    let handle = OwnedObjectPath::try_from(handle)?;
    let shortcuts: Vec<(&str, HashMap<&str, Value>)> = Action::ALL
        .iter()
        .map(|a| (a.id(), HashMap::from([("description", Value::from(a.description())), ("preferred_trigger", Value::from(a.preferred()))])))
        .collect();
    let options = HashMap::from([("handle_token", Value::from("gameviber_bind"))]);
    let results = request(&conn, "BindShortcuts", "gameviber_bind", &(&handle, shortcuts, "", options))?;
    let bound: Bound = results.get("shortcuts").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok()).unwrap_or_default();
    *status.lock().unwrap() = Status::Ready(triggers(&bound));
    *session.lock().unwrap() = Some((conn.clone(), handle.clone()));
    log::info!("keyboard shortcuts ready");

    let portal = Proxy::new(&conn, PORTAL, PORTAL_PATH, SHORTCUTS)?;
    // The keys changed in the desktop's settings.
    let changes = portal.receive_signal("ShortcutsChanged")?;
    let (status, watched) = (status.clone(), handle.clone());
    std::thread::spawn(move || {
        for message in changes {
            if let Ok((path, bound)) = message.body().deserialize::<(OwnedObjectPath, Bound)>() {
                if path == watched {
                    *status.lock().unwrap() = Status::Ready(triggers(&bound));
                }
            }
        }
    });
    for message in portal.receive_signal("Activated")? {
        let Ok((path, id, _, _)) = message.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>() else { continue };
        if path != handle {
            continue;
        }
        if let Some(action) = Action::ALL.iter().find(|a| a.id() == id) {
            log::info!("shortcut: {}", action.description());
            if tx.send(*action).is_err() {
                break;
            }
        }
    }
    Ok(())
}
