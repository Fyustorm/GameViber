//! Linux: the desktop's global shortcuts portal
//! (`org.freedesktop.portal.GlobalShortcuts`): the desktop owns the keys, so
//! the game never sees them, and no input device permission is needed. The
//! desktop asks the player to confirm the keys the first time, and lets them
//! change them later.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::{Action, Status, APP_ID};
use crate::platform::linux::portal::{PORTAL, PORTAL_PATH};

const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
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

/// A request to the shortcuts portal; refusing it is an error.
fn request<B>(conn: &Connection, method: &str, token: &str, body: &B) -> anyhow::Result<HashMap<String, OwnedValue>>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    crate::platform::linux::portal::request(conn, SHORTCUTS, method, token, body)?.with_context(|| format!("{method}: refused"))
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
    if let Err(e) = crate::platform::linux::desktop::desktop_entry() {
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
