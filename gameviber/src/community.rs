//! The community server (`server/`): the modes players published, by game, to
//! install and keep up to date, and publishing one's own, privately (shared
//! by a code) or for everyone. Calls block (`ureq`): the GUI makes them in
//! threads of their own.
//!
//! A mode installed from the community, or published from here, keeps where
//! it came from in its package (`community.json`, `Origin`): its id on the
//! server, its version, and its script as installed, to tell whether the
//! player changed it since (an update then installs beside it).

use std::fs;
use std::io::Read;
use std::time::Duration;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{self, ModeEntry};
use crate::sharing;

/// The community server, chosen at build time (`GAMEVIBER_COMMUNITY_URL`).
pub const URL: &str = match option_env!("GAMEVIBER_COMMUNITY_URL") {
    Some(url) => url,
    None => "http://localhost:8080",
};
const ORIGIN_FILE: &str = "community.json";
const ACCOUNT_FILE: &str = "community.toml";
const LINKS_FILE: &str = "community-links.json";
/// The mode API versions this GameViber runs (`mode::API_VERSION`).
pub const APIS: [u32; 1] = [crate::mode::API_VERSION];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameView {
    pub id: i64,
    pub name: String,
    pub steam_app_id: Option<u32>,
    /// Its public modes.
    pub modes: u64,
}

/// What players who share their stats make of a mode (`server/`, `Stats`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Figures {
    /// Installations that played it in the last 30 days.
    pub players: u64,
    pub median_minutes: u64,
    /// Share of its players who played it 3 sessions or more, 0..1.
    pub came_back: f64,
    pub likes: u64,
    pub dislikes: u64,
}

impl Figures {
    /// "92 % liked", once a few voted.
    pub fn liked(&self) -> Option<String> {
        let votes = self.likes + self.dislikes;
        (votes > 0).then(|| format!("{:.0} % liked", 100.0 * self.likes as f64 / votes as f64))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
    /// Its latest version, and that version's mode API.
    pub version: u32,
    pub api: u32,
    pub updated_at: String,
    #[serde(default)]
    pub figures: Figures,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionView {
    pub number: u32,
    pub changelog: String,
    pub api: u32,
    pub size: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub game: GameView,
    pub author: String,
    pub downloads: u64,
    /// Newest first.
    pub versions: Vec<VersionView>,
    pub updated_at: String,
    /// Only for its author.
    pub visibility: Option<String>,
    pub share_code: Option<String>,
    pub withdrawn_at: Option<String>,
    pub withdrawn_reason: Option<String>,
    #[serde(default)]
    pub figures: Figures,
}

impl ModeDetail {
    pub fn latest(&self) -> Option<&VersionView> {
        self.versions.first()
    }

    pub fn is_public(&self) -> bool {
        self.visibility.as_deref() == Some("public")
    }
}

/// How a mode is reached: by its id (a public one) or by its share code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Id(String),
    Code(String),
}

impl Source {
    fn path(&self) -> String {
        match self {
            Source::Id(id) => format!("/api/modes/{id}"),
            Source::Code(code) => format!("/api/shared/{}", code.trim()),
        }
    }
}

/// Where a mode installed from the community, or published from here, comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    /// Its id on the server, and the code it was installed with (private modes).
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The version installed or published.
    pub version: u32,
    /// Published by the player (their account then manages it).
    #[serde(default)]
    pub own: bool,
    /// The script as installed: a different one now was changed by the player.
    pub script_sha256: String,
    /// The player keeps this version: no update is offered.
    #[serde(default)]
    pub pinned: bool,
    /// What the player said of it: 1 liked, -1 not, 0 nothing.
    #[serde(default)]
    pub vote: i8,
}

impl Origin {
    /// Where the mode `entry` (or the mode it is a variant of) comes from, if from the community.
    pub fn of(entry: &ModeEntry) -> Option<Origin> {
        let text = fs::read_to_string(entry.dir()?.join(ORIGIN_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, entry: &ModeEntry) -> anyhow::Result<()> {
        let dir = entry.dir().context("only a package comes from the community")?;
        config::write_file(&dir.join(ORIGIN_FILE), &serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn source(&self) -> Source {
        match &self.code {
            Some(code) => Source::Code(code.clone()),
            None => Source::Id(self.id.clone()),
        }
    }

    /// The player changed the mode's script since it was installed.
    pub fn changed(&self, entry: &ModeEntry) -> bool {
        let main = ModeEntry::from_id(&entry.main_id());
        main.source().map(|s| script_sha256(&s) != self.script_sha256).unwrap_or(false)
    }
}

fn script_sha256(source: &str) -> String {
    Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

const USAGE_FILE: &str = "usage.json";
/// Play time with a mode of one's own, unchanged, after which publishing it is suggested.
pub const SUGGEST_AFTER_SECS: f64 = 3.0 * 3600.0;

/// How long a mode of the player's was played since its script last changed,
/// to suggest publishing a mode that works (`usage.json` in its package).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub played_secs: f64,
    pub script_sha256: String,
    /// "Later": suggested again past this play time.
    pub remind_at_secs: f64,
    /// "Don't ask for this mode".
    pub dismissed: bool,
}

impl Usage {
    pub fn of(entry: &ModeEntry) -> Usage {
        let Some(dir) = entry.dir() else { return Usage::default() };
        fs::read_to_string(dir.join(USAGE_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, entry: &ModeEntry) {
        let Some(dir) = entry.dir() else { return };
        if let Err(e) = serde_json::to_string(self).map_err(std::io::Error::other).and_then(|t| config::write_file(&dir.join(USAGE_FILE), &t)) {
            log::warn!("cannot save how long {} was played: {e}", entry.key);
        }
    }

    /// Publishing the mode is worth suggesting now.
    pub fn suggest(&self) -> bool {
        !self.dismissed && self.played_secs >= SUGGEST_AFTER_SECS.max(self.remind_at_secs)
    }
}

/// `secs` more of play with the mode `mode` (a user mode); the count starts
/// over when its script changed.
pub fn record_play(mode: &str, secs: f64) {
    let entry = ModeEntry::from_id(&ModeEntry::from_id(mode).main_id());
    if entry.dir().is_none() {
        return;
    }
    let Ok(source) = entry.source() else { return };
    let sha = script_sha256(&source);
    let mut usage = Usage::of(&entry);
    if usage.script_sha256 != sha {
        usage = Usage { script_sha256: sha, dismissed: usage.dismissed, ..Usage::default() };
    }
    usage.played_secs += secs;
    usage.save(&entry);
}

/// Play time with the modes installed from the community, gathered and sent
/// now and then when the player shares their stats.
#[derive(Debug, Default)]
pub struct PlayReport {
    /// By id on the server: seconds, sessions.
    pending: std::collections::HashMap<String, (f64, u32)>,
    sent: Option<std::time::Instant>,
}

/// Plays are sent at most this often.
const REPORT_EVERY: Duration = Duration::from_secs(10 * 60);

impl PlayReport {
    /// `secs` more with the mode `mode`, and one more session when `session`;
    /// nothing for a mode not installed from the community (or the player's own).
    pub fn add(&mut self, mode: &str, secs: f64, session: bool) {
        let entry = ModeEntry::from_id(&ModeEntry::from_id(mode).main_id());
        let Some(origin) = Origin::of(&entry).filter(|o| !o.own) else { return };
        let pending = self.pending.entry(origin.id).or_default();
        pending.0 += secs;
        pending.1 += u32::from(session);
    }

    /// Sends what was gathered, in a thread, unless it was sent lately (`now`:
    /// whatever the time); the thread, to wait for it before quitting.
    pub fn send(&mut self, url: &str, installation: &str, now: bool) -> Option<std::thread::JoinHandle<()>> {
        if self.pending.is_empty() || (!now && self.sent.is_some_and(|t| t.elapsed() < REPORT_EVERY)) {
            return None;
        }
        self.sent = Some(std::time::Instant::now());
        let plays: Vec<(String, u64, u32)> = self.pending.drain().map(|(id, (secs, sessions))| (id, secs.round() as u64, sessions)).collect();
        let (client, installation) = (Client::new(url, None), installation.to_owned());
        Some(std::thread::spawn(move || {
            if let Err(e) = client.send_plays(&installation, &plays) {
                log::warn!("cannot send play time to the community: {e:#}");
            }
        }))
    }
}

/// The player's author account on the server: their pseudo and its token.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// The server it belongs to.
    pub url: String,
    pub pseudo: String,
    pub token: String,
}

impl Account {
    fn path() -> std::path::PathBuf {
        config::config_dir().join(ACCOUNT_FILE)
    }

    /// The account signed in on the server `url`, if any.
    pub fn load(url: &str) -> Option<Account> {
        let text = fs::read_to_string(Self::path()).ok()?;
        toml::from_str::<Account>(&text).ok().filter(|a| a.url == url && !a.token.is_empty())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        config::write_file(&Self::path(), &toml::to_string(self)?)?;
        // Only the player reads their token.
        crate::platform::keep_private(&Self::path());
        Ok(())
    }

    pub fn sign_out() {
        let _ = fs::remove_file(Self::path());
    }
}

/// A connection to the server at `url`, signed in when `token` is set.
#[derive(Debug, Clone)]
pub struct Client {
    url: String,
    token: Option<String>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

#[derive(Deserialize)]
struct Session {
    token: String,
    pseudo: String,
}

impl Client {
    pub fn new(url: &str, account: Option<&Account>) -> Self {
        Client { url: url.trim_end_matches('/').to_owned(), token: account.map(|a| a.token.clone()) }
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .into()
    }

    /// Sends a request built by `send`; the answer's body, or the server's error.
    fn call(&self, path: &str, send: impl FnOnce(&ureq::Agent, &str, Option<String>) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}{path}", self.url);
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut response = send(&Self::agent(), &url, auth).with_context(|| format!("the community server ({}) cannot be reached", self.url))?;
        let status = response.status();
        let mut body = Vec::new();
        response.body_mut().as_reader().take(128 << 20).read_to_end(&mut body)?;
        if status.is_success() {
            return Ok(body);
        }
        match serde_json::from_slice::<ErrorBody>(&body) {
            Ok(e) => bail!("{}", e.error),
            Err(_) if status == 401 => bail!("sign in again"),
            Err(_) => bail!("the community server answered {status}"),
        }
    }

    fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> anyhow::Result<T> {
        let body = self.call(path, |agent, url, auth| {
            let request = agent.get(url);
            match auth {
                Some(auth) => request.header("Authorization", &auth).call(),
                None => request.call(),
            }
        })?;
        serde_json::from_slice(&body).context("the community server's answer")
    }

    fn send_json<T: for<'de> Deserialize<'de>>(&self, method: &str, path: &str, json: &serde_json::Value) -> anyhow::Result<Option<T>> {
        let text = json.to_string();
        let body = self.call(path, |agent, url, auth| {
            let request = match method {
                "PATCH" => agent.patch(url),
                _ => agent.post(url),
            };
            let request = request.header("Content-Type", "application/json");
            match auth {
                Some(auth) => request.header("Authorization", &auth).send(text.as_bytes()),
                None => request.send(text.as_bytes()),
            }
        })?;
        if body.is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&body).context("the community server's answer")?))
    }

    fn send_empty<T: for<'de> Deserialize<'de>>(&self, method: &str, path: &str) -> anyhow::Result<T> {
        let body = self.call(path, |agent, url, auth| {
            let auth = auth.unwrap_or_default();
            match method {
                "DELETE" => agent.delete(url).header("Authorization", &auth).call(),
                _ => agent.post(url).header("Authorization", &auth).send_empty(),
            }
        })?;
        serde_json::from_slice(&body).context("the community server's answer")
    }

    fn multipart<T: for<'de> Deserialize<'de>>(&self, path: &str, fields: &[(&str, &str)], package: &[u8]) -> anyhow::Result<T> {
        let (content_type, form) = multipart_form(fields, package);
        let body = self.call(path, |agent, url, auth| {
            agent.post(url).header("Authorization", &auth.unwrap_or_default()).header("Content-Type", &content_type).send(&form[..])
        })?;
        serde_json::from_slice(&body).context("the community server's answer")
    }

    /// Games with public modes, the most first; `search` in their name.
    pub fn games(&self, search: &str) -> anyhow::Result<Vec<GameView>> {
        self.get(&format!("/api/games?search={}", encode(search)))
    }

    /// The game a player plays on the server, when it has public modes.
    pub fn game_match(&self, name: &str, steam_app_id: Option<u32>) -> anyhow::Result<Option<GameView>> {
        let app = steam_app_id.map(|a| format!("&steamAppId={a}")).unwrap_or_default();
        match self.get(&format!("/api/games/match?name={}{app}", encode(name))) {
            Ok(game) => Ok(Some(game)),
            Err(e) if e.to_string().contains("no public mode") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A game's public modes: `sort` "new" or "downloads".
    pub fn modes(&self, game: i64, sort: &str) -> anyhow::Result<Vec<ModeSummary>> {
        self.get(&format!("/api/games/{game}/modes?sort={sort}"))
    }

    pub fn mode(&self, source: &Source) -> anyhow::Result<ModeDetail> {
        self.get(&source.path())
    }

    /// A version's package (the latest without `version`), counted as a download.
    pub fn download(&self, source: &Source, version: Option<u32>) -> anyhow::Result<Vec<u8>> {
        let query = version.map(|v| format!("?version={v}")).unwrap_or_default();
        self.call(&format!("{}/package{query}", source.path()), |agent, url, _| agent.get(url).call())
    }

    /// Creates an author account, or signs in to one.
    pub fn sign_in(&self, pseudo: &str, password: &str, new: bool) -> anyhow::Result<Account> {
        let path = if new { "/api/authors" } else { "/api/authors/session" };
        let session: Session = self
            .send_json("POST", path, &serde_json::json!({ "pseudo": pseudo, "password": password }))?
            .context("the community server's answer")?;
        Ok(Account { url: self.url.clone(), pseudo: session.pseudo, token: session.token })
    }

    /// The modes of the author signed in.
    pub fn my_modes(&self) -> anyhow::Result<Vec<ModeDetail>> {
        self.get("/api/authors/me/modes")
    }

    pub fn publish(&self, package: &[u8], name: &str, description: &str, public: bool, changelog: &str) -> anyhow::Result<ModeDetail> {
        let visibility = if public { "public" } else { "private" };
        let fields = [("name", name), ("description", description), ("visibility", visibility), ("changelog", changelog)];
        self.multipart("/api/modes", &fields, package)
    }

    pub fn publish_version(&self, id: &str, package: &[u8], changelog: &str) -> anyhow::Result<ModeDetail> {
        self.multipart(&format!("/api/modes/{id}/versions"), &[("changelog", changelog)], package)
    }

    /// Its name, description or visibility (what is None stays).
    pub fn change(&self, id: &str, name: Option<&str>, description: Option<&str>, public: Option<bool>) -> anyhow::Result<ModeDetail> {
        let visibility = public.map(|p| if p { "public" } else { "private" });
        let json = serde_json::json!({ "name": name, "description": description, "visibility": visibility });
        self.send_json("PATCH", &format!("/api/modes/{id}"), &json)?.context("the community server's answer")
    }

    pub fn new_share_code(&self, id: &str) -> anyhow::Result<ModeDetail> {
        self.send_empty("POST", &format!("/api/modes/{id}/share-code"))
    }

    pub fn withdraw(&self, id: &str) -> anyhow::Result<ModeDetail> {
        self.send_empty("DELETE", &format!("/api/modes/{id}"))
    }

    /// Play time with modes installed from the community, under this installation's id.
    pub fn send_plays(&self, installation: &str, plays: &[(String, u64, u32)]) -> anyhow::Result<()> {
        let plays: Vec<_> = plays.iter().map(|(mode, seconds, sessions)| serde_json::json!({ "mode": mode, "seconds": seconds, "sessions": sessions })).collect();
        self.send_json::<serde_json::Value>("POST", "/api/stats/plays", &serde_json::json!({ "installation": installation, "plays": plays }))?;
        Ok(())
    }

    /// 1 liked, -1 not, 0 taken back: only for a mode played (with stats shared).
    pub fn vote(&self, installation: &str, id: &str, value: i8) -> anyhow::Result<()> {
        self.send_json::<serde_json::Value>("POST", "/api/stats/votes", &serde_json::json!({ "installation": installation, "mode": id, "value": value }))?;
        Ok(())
    }

    /// `reason`: "broken", "content" or "other".
    pub fn report(&self, id: &str, reason: &str, details: &str) -> anyhow::Result<()> {
        self.send_json::<serde_json::Value>("POST", &format!("/api/modes/{id}/reports"), &serde_json::json!({ "reason": reason, "details": details }))?;
        Ok(())
    }
}

/// A form with `fields` and the file `package`, and its content type.
fn multipart_form(fields: &[(&str, &str)], package: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("gameviber-{:016x}", rand_u64());
    let mut out = Vec::new();
    for (name, value) in fields {
        out.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    out.extend(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"mode.gameviber\"\r\nContent-Type: application/zip\r\n\r\n")
            .as_bytes(),
    );
    out.extend(package);
    out.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), out)
}

/// A random id for this installation: 32 hexadecimal digits.
pub fn new_installation_id() -> String {
    format!("{:016x}{:016x}", rand_u64(), rand_u64().rotate_left(17) ^ std::process::id() as u64)
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.finish()
}

/// `text` for a URL's query.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Installs a mode from the community: its package is imported (`sharing`),
/// and remembers where it comes from. Returns what the import did.
pub fn install(client: &Client, source: &Source, detail: &ModeDetail) -> anyhow::Result<sharing::Imported> {
    let version = detail.latest().context("this mode has no version")?;
    if !APIS.contains(&version.api) {
        bail!("this mode needs a newer GameViber (mode API {})", version.api);
    }
    let file = temp_file(&client.download(source, None)?)?;
    let imported = sharing::import(&file);
    let _ = fs::remove_file(&file);
    let imported = imported?;
    let entry = ModeEntry::from_id(&imported.mode);
    // The very same script is a mode of the player's already: it stays theirs,
    // only linked to the community's, which then opens it.
    if imported.mode_existed && Origin::of(&entry).is_none() {
        let mut links = links();
        links.insert(detail.id.clone(), imported.mode.clone());
        config::write_file(&config::config_dir().join(LINKS_FILE), &serde_json::to_string(&links)?)?;
        return Ok(imported);
    }
    let code = match source {
        Source::Code(code) => Some(code.trim().to_uppercase()),
        Source::Id(_) => None,
    };
    let script = entry.source()?;
    Origin { id: detail.id.clone(), code, version: version.number, own: false, script_sha256: script_sha256(&script), pinned: false, vote: 0 }.save(&entry)?;
    Ok(imported)
}

/// Modes of the community the player has as modes of their own (the very
/// same script): their id on the server, and the player's mode.
pub fn links() -> std::collections::HashMap<String, String> {
    fs::read_to_string(config::config_dir().join(LINKS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Brings a mode installed from the community to the latest version. One the
/// player changed is kept: the new version is installed beside it (its id is
/// returned then).
pub fn update(client: &Client, mode: &str) -> anyhow::Result<Option<String>> {
    let entry = ModeEntry::from_id(&ModeEntry::from_id(mode).main_id());
    let origin = Origin::of(&entry).context("this mode does not come from the community")?;
    let detail = client.mode(&origin.source())?;
    if entry.dir().is_some() && origin.changed(&entry) {
        let imported = install(client, &origin.source(), &detail)?;
        return Ok(Some(imported.mode));
    }
    let version = detail.latest().context("this mode has no version")?;
    if !APIS.contains(&version.api) {
        bail!("its new version needs a newer GameViber (mode API {})", version.api);
    }
    let file = temp_file(&client.download(&origin.source(), None)?)?;
    let updated = sharing::update(&file, &entry.id);
    let _ = fs::remove_file(&file);
    updated?;
    let script = entry.source()?;
    Origin { version: version.number, script_sha256: script_sha256(&script), ..origin }.save(&entry)?;
    Ok(None)
}

/// Publishes the mode `mode` of the game `game` (by ids), without the
/// captures `left_out`; it then remembers it as the player's.
pub fn publish(
    client: &Client,
    game: &str,
    mode: &str,
    name: &str,
    description: &str,
    public: bool,
    changelog: &str,
    left_out: &[String],
) -> anyhow::Result<ModeDetail> {
    let entry = ModeEntry::from_id(&ModeEntry::from_id(mode).main_id());
    let package = exported(game, &entry, left_out)?;
    let detail = match Origin::of(&entry).filter(|o| o.own) {
        Some(origin) => client.publish_version(&origin.id, &package, changelog)?,
        None => client.publish(&package, name, description, public, changelog)?,
    };
    let script = entry.source()?;
    let version = detail.latest().map_or(1, |v| v.number);
    Origin { id: detail.id.clone(), code: None, version, own: true, script_sha256: script_sha256(&script), pinned: false, vote: 0 }.save(&entry)?;
    Ok(detail)
}

/// The mode `entry` with its game as a `.gameviber` file's bytes.
fn exported(game: &str, entry: &ModeEntry, left_out: &[String]) -> anyhow::Result<Vec<u8>> {
    let path = std::env::temp_dir().join(format!("gameviber-publish-{}.gameviber", rand_u64()));
    let result = sharing::export(game, &entry.id, &path, left_out).and_then(|()| Ok(fs::read(&path)?));
    let _ = fs::remove_file(&path);
    result
}

fn temp_file(bytes: &[u8]) -> anyhow::Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("gameviber-community-{}.gameviber", rand_u64()));
    fs::write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_and_queries_are_encoded() {
        let (content_type, form) = multipart_form(&[("name", "Battle pulse")], b"PK");
        let boundary = content_type.strip_prefix("multipart/form-data; boundary=").unwrap();
        let text = String::from_utf8_lossy(&form);
        assert!(text.starts_with(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nBattle pulse\r\n")), "{text}");
        assert!(text.contains("name=\"package\"; filename=\"mode.gameviber\"\r\nContent-Type: application/zip\r\n\r\nPK\r\n"), "{text}");
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
        assert_eq!(encode("Metaphor: ReFantazio"), "Metaphor%3A%20ReFantazio");
        assert_eq!(Source::Code(" GV-AB12-CD34 ".into()).path(), "/api/shared/GV-AB12-CD34");
    }

    #[test]
    fn publishing_is_suggested_for_a_mode_played_unchanged() {
        let root = std::env::temp_dir().join(format!("gameviber-usage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        config::MODES_TEST_DIR.with(|d| *d.borrow_mut() = Some(root.join("modes")));
        let script = config::unused_mode_path("mine");
        config::write_file(&script, "-- v1").unwrap();
        let mode = script.to_string_lossy().into_owned();
        let entry = ModeEntry::from_id(&mode);
        record_play(&mode, SUGGEST_AFTER_SECS - 60.0);
        assert!(!Usage::of(&entry).suggest());
        record_play(&mode, 60.0);
        assert!(Usage::of(&entry).suggest(), "played long enough");
        // Changed: the count starts over.
        config::write_file(&script, "-- v2").unwrap();
        record_play(&mode, 60.0);
        assert_eq!(Usage::of(&entry).played_secs, 60.0);
        // Later: once played as long again.
        record_play(&mode, SUGGEST_AFTER_SECS);
        let usage = Usage::of(&entry);
        Usage { remind_at_secs: usage.played_secs + SUGGEST_AFTER_SECS, ..usage }.save(&entry);
        assert!(!Usage::of(&entry).suggest());
        // A mode from the community counts in play reports, the player's own does not.
        let mut report = PlayReport::default();
        report.add(&mode, 60.0, true);
        assert!(report.pending.is_empty());
        Origin { id: "x".into(), code: None, version: 1, own: false, script_sha256: String::new(), pinned: false, vote: 0 }.save(&entry).unwrap();
        report.add(&mode, 60.0, true);
        report.add(&mode, 30.0, false);
        assert_eq!(report.pending["x"], (90.0, 1));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn server_answers_are_read() {
        let detail: ModeDetail = serde_json::from_str(
            r#"{"id":"k3x9m2q7ab","name":"Battle pulse","description":"","game":{"id":1,"name":"Hades II","steamAppId":1145350,"modes":2},
                "author":"Fyu","downloads":3,"versions":[{"number":2,"changelog":"Faster","api":1,"size":10,"sha256":"x","createdAt":"2026-10-06T20:19:47Z"}],
                "createdAt":"2026-10-06T20:19:47Z","updatedAt":"2026-10-06T20:19:47Z","visibility":null,"shareCode":null,"withdrawnAt":null,"withdrawnReason":null}"#,
        )
        .unwrap();
        assert_eq!((detail.latest().unwrap().number, detail.game.steam_app_id, detail.is_public()), (2, Some(1145350), false));
    }
}


