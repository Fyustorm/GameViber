//! The desktop's portals (`org.freedesktop.portal.*`, over the session bus):
//! global shortcuts (`shortcuts/linux.rs`) and file dialogs (`files.rs`).

use std::collections::HashMap;

use anyhow::{bail, Context};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

pub const PORTAL: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// A portal request: calls `method` of `interface` (whose options carry
/// `token` as their handle token), and waits for its response. None: the
/// player cancelled it.
pub fn request<B>(
    conn: &Connection,
    interface: &str,
    method: &str,
    token: &str,
    body: &B,
) -> anyhow::Result<Option<HashMap<String, OwnedValue>>>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let sender = conn.unique_name().context("no bus name")?.trim_start_matches(':').replace('.', "_");
    let path = format!("{PORTAL_PATH}/request/{sender}/{token}");
    let request = Proxy::new(conn, PORTAL, path, "org.freedesktop.portal.Request")?;
    // Listening before calling, not to miss a quick response.
    let mut responses = request.receive_signal("Response")?;
    conn.call_method(Some(PORTAL), PORTAL_PATH, Some(interface), method, body).with_context(|| method.to_owned())?;
    let message = responses.next().context("no response")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
    match code {
        0 => Ok(Some(results)),
        1 => Ok(None),
        _ => bail!("{method}: failed"),
    }
}
