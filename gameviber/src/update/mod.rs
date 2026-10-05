//! Updates: the newest GameViber released on GitHub, and installing it the
//! way this GameViber was installed (`Installation`). Our own packages and
//! archive are downloaded, checked against the release's SHA256SUMS and
//! installed (`linux`: through the system's package manager, or by replacing
//! the archive's files); a GameViber a store or a package repository updates,
//! or one built from source, is only told about new versions.
//!
//! Pre-releases are offered while this GameViber is one (alpha, beta).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
use unsupported as backend;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config;

/// The GitHub repository releases are published in.
pub const REPOSITORY: &str = "Fyustorm/GameViber";
/// Releases are looked for this long after starting, then this often.
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// The release files listing every file's SHA-256.
const SUMS: &str = "SHA256SUMS";

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is semver")
}

/// How this GameViber was installed, which decides how it updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installation {
    /// One of our packages, installed by hand from a release.
    Package(PackageKind),
    /// Our archive, extracted into this directory.
    Archive(PathBuf),
    /// Updated by a store or a package repository (named): only told about new versions.
    Managed(String),
    /// Built from source.
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    Deb,
    Rpm,
    Arch,
}

impl Installation {
    pub fn describe(&self) -> String {
        match self {
            Installation::Package(PackageKind::Deb) => "Debian / Ubuntu package".into(),
            Installation::Package(PackageKind::Rpm) => "RPM package".into(),
            Installation::Package(PackageKind::Arch) => "Arch package".into(),
            Installation::Archive(dir) => format!("archive in {}", dir.display()),
            Installation::Managed(by) => format!("updated by {by}"),
            Installation::Source => "built from source".into(),
        }
    }

    /// GameViber can download and install updates itself.
    pub fn installs_updates(&self) -> bool {
        matches!(self, Installation::Package(_) | Installation::Archive(_))
    }

    /// The release file to install (None: GameViber does not install updates).
    fn asset_suffix(&self) -> Option<&'static str> {
        Some(match self {
            Installation::Package(PackageKind::Deb) => ".deb",
            Installation::Package(PackageKind::Rpm) => ".rpm",
            Installation::Package(PackageKind::Arch) => ".pkg.tar.zst",
            Installation::Archive(_) => ".tar.gz",
            Installation::Managed(_) | Installation::Source => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    /// The release notes (Markdown).
    pub notes: String,
    /// Its page on GitHub.
    pub page: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Where updates stand, for the GUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Phase {
    /// Not checked yet (or checks are off).
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { release: Release, done: u64, total: u64 },
    /// Waiting for the password, then installing.
    Installing(Release),
    /// Installed: takes effect when GameViber restarts.
    Installed(Release),
    /// The check or the installation failed; the release, if one was found.
    Failed { release: Option<Release>, error: String },
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub phase: Phase,
    pub installation: Option<Installation>,
    pub last_check: Option<Instant>,
}

enum Request {
    Check,
    Install,
    SetAutomatic(bool),
}

/// Checks for updates on a thread of its own, and installs them when asked.
pub struct Updater {
    status: Arc<Mutex<Status>>,
    requests: Sender<Request>,
}

impl Updater {
    /// `automatic`: checks a little after starting, then every few hours.
    pub fn start(automatic: bool) -> Self {
        let status = Arc::new(Mutex::new(Status::default()));
        let (requests, rx) = mpsc::channel();
        let shared = status.clone();
        std::thread::Builder::new()
            .name("updates".into())
            .spawn(move || {
                let installation = backend::installation();
                log::info!("GameViber {} ({})", current_version(), installation.describe());
                shared.lock().unwrap().installation = Some(installation.clone());
                run(&shared, &rx, &installation, automatic);
            })
            .expect("spawn the update thread");
        Self { status, requests }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn check(&self) {
        let _ = self.requests.send(Request::Check);
    }

    /// Downloads and installs the release found (for installations that can).
    pub fn install(&self) {
        let _ = self.requests.send(Request::Install);
    }

    pub fn set_automatic(&self, on: bool) {
        let _ = self.requests.send(Request::SetAutomatic(on));
    }
}

fn run(status: &Mutex<Status>, rx: &mpsc::Receiver<Request>, installation: &Installation, mut automatic: bool) {
    let set = |phase: Phase| status.lock().unwrap().phase = phase;
    let mut next_check = Instant::now() + FIRST_CHECK;
    loop {
        let wait = if automatic { next_check.saturating_duration_since(Instant::now()) } else { Duration::from_secs(3600) };
        let request = match rx.recv_timeout(wait) {
            Ok(request) => request,
            Err(RecvTimeoutError::Timeout) if automatic => Request::Check,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match request {
            Request::SetAutomatic(on) => automatic = on,
            Request::Check => {
                // A downloaded update is not checked over.
                if matches!(status.lock().unwrap().phase, Phase::Downloading { .. } | Phase::Installing(_) | Phase::Installed(_)) {
                    continue;
                }
                next_check = Instant::now() + CHECK_EVERY;
                set(Phase::Checking);
                let phase = match newest_release() {
                    Ok(Some(release)) => {
                        log::info!("GameViber {} is available", release.version);
                        Phase::Available(release)
                    }
                    Ok(None) => Phase::UpToDate,
                    Err(e) => {
                        log::warn!("cannot check for updates: {e:#}");
                        Phase::Failed { release: None, error: format!("{e:#}") }
                    }
                };
                let mut s = status.lock().unwrap();
                s.phase = phase;
                s.last_check = Some(Instant::now());
            }
            Request::Install => {
                let release = match &status.lock().unwrap().phase {
                    Phase::Available(release) | Phase::Failed { release: Some(release), .. } => release.clone(),
                    _ => continue,
                };
                match install(status, installation, &release, &config::data_dir().join("updates")) {
                    Ok(()) => {
                        log::info!("GameViber {} installed: restart to use it", release.version);
                        set(Phase::Installed(release));
                    }
                    Err(e) => {
                        log::warn!("cannot install GameViber {}: {e:#}", release.version);
                        set(Phase::Failed { release: Some(release), error: format!("{e:#}") });
                    }
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into()
}

/// Where releases are listed: GitHub's API, or `GAMEVIBER_RELEASES_URL` (a
/// mirror, or a test server) answering the same.
fn releases_url() -> String {
    std::env::var("GAMEVIBER_RELEASES_URL")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=30"))
}

/// The newest release above this version, if any.
fn newest_release() -> anyhow::Result<Option<Release>> {
    let url = releases_url();
    let body = agent()
        .get(&url)
        .header("User-Agent", &format!("GameViber/{}", current_version()))
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("GitHub")?
        .into_body()
        .read_to_string()?;
    let releases: Vec<GithubRelease> = serde_json::from_str(&body).context("GitHub's answer")?;
    Ok(pick(releases, &current_version()))
}

/// The newest release above `current`: published, and a pre-release only
/// while `current` is one.
fn pick(releases: Vec<GithubRelease>, current: &Version) -> Option<Release> {
    let prereleases = !current.pre.is_empty();
    releases
        .into_iter()
        .filter(|r| !r.draft && (prereleases || !r.prerelease))
        .filter_map(|r| {
            let version = Version::parse(r.tag_name.trim_start_matches('v')).ok()?;
            (prereleases || version.pre.is_empty()).then(|| Release {
                version,
                notes: r.body.unwrap_or_default(),
                page: r.html_url,
                assets: r.assets,
            })
        })
        .filter(|r| r.version > *current)
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Downloads the release's file for this installation into `dir`, checks it,
/// and installs it.
fn install(status: &Mutex<Status>, installation: &Installation, release: &Release, dir: &Path) -> anyhow::Result<()> {
    let suffix = installation.asset_suffix().context("this installation is updated by other means")?;
    let asset = release.assets.iter().find(|a| a.name.ends_with(suffix)).with_context(|| format!("the release has no {suffix} file"))?;
    let sums = release.assets.iter().find(|a| a.name == SUMS).context("the release has no SHA256SUMS")?;
    // Earlier downloads are not kept.
    let _ = std::fs::remove_dir_all(dir);
    config::create_dir(dir)?;

    let sums = String::from_utf8(download(sums, dir, |_| {})?.1)?;
    let expected = expected_sum(&sums, &asset.name).with_context(|| format!("{} is not in SHA256SUMS", asset.name))?;
    let progress = |done| status.lock().unwrap().phase = Phase::Downloading { release: release.clone(), done, total: asset.size };
    let (path, sum) = download(asset, dir, progress)?;
    anyhow::ensure!(hex(&sum) == expected, "{} is damaged (its SHA-256 does not match)", asset.name);

    status.lock().unwrap().phase = Phase::Installing(release.clone());
    backend::apply(installation, &path)?;
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// Downloads `asset` into `dir`: its path, and its SHA-256 (or, for small
/// files, its content).
fn download(asset: &Asset, dir: &Path, progress: impl Fn(u64)) -> anyhow::Result<(PathBuf, Vec<u8>)> {
    let path = dir.join(&asset.name);
    let response = agent()
        .get(&asset.browser_download_url)
        .header("User-Agent", &format!("GameViber/{}", current_version()))
        .config()
        .timeout_global(None)
        .build()
        .call()
        .with_context(|| format!("downloading {}", asset.name))?;
    let mut body = response.into_body().into_reader();
    let mut file = std::fs::File::create(&path)?;
    let mut hasher = Sha256::new();
    let mut small = Vec::new();
    let mut buffer = vec![0; 1 << 16];
    let mut done = 0;
    loop {
        let n = body.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])?;
        hasher.update(&buffer[..n]);
        if asset.name == SUMS {
            small.extend_from_slice(&buffer[..n]);
        }
        done += n as u64;
        progress(done);
    }
    file.flush()?;
    anyhow::ensure!(done == asset.size, "{}: got {done} bytes instead of {}", asset.name, asset.size);
    Ok((path, if asset.name == SUMS { small } else { hasher.finalize().to_vec() }))
}

/// The SHA-256 of `name` in a `sha256sum` listing.
/// GitHub replaces the `~` of file names (deb and rpm prerelease versions)
/// with a dot: SHA256SUMS of the first releases still has the `~`.
fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*').replace('~', ".") == name.replace('~', ".")).then(|| sum.to_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

static RESTART: AtomicBool = AtomicBool::new(false);

/// GameViber starts again once the GUI is closed (after an update).
pub fn request_restart() {
    RESTART.store(true, Ordering::Relaxed);
}

/// Called last: starts the updated GameViber in place of this one, if asked.
pub fn restart_if_requested() {
    if RESTART.load(Ordering::Relaxed) {
        let e = backend::relaunch();
        log::error!("cannot start GameViber again: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, draft: bool) -> GithubRelease {
        GithubRelease { tag_name: tag.into(), draft, prerelease, body: None, html_url: String::new(), assets: Vec::new() }
    }

    fn picked(releases: Vec<GithubRelease>, current: &str) -> Option<String> {
        pick(releases, &Version::parse(current).unwrap()).map(|r| r.version.to_string())
    }

    #[test]
    fn the_newest_release_is_picked() {
        let all = || {
            vec![
                release("v0.1.0-alpha.1", true, false),
                release("v0.1.0-alpha.3", true, false),
                release("v0.1.0-alpha.2", true, false),
                release("v0.1.0-alpha.4", true, true),
                release("nightly", true, false),
            ]
        };
        assert_eq!(picked(all(), "0.1.0-alpha.1").as_deref(), Some("0.1.0-alpha.3"), "drafts are left out");
        assert_eq!(picked(all(), "0.1.0-alpha.3"), None);
        assert_eq!(picked(all(), "0.1.0"), None, "no pre-release once on a stable version");
        let mut with_stable = all();
        with_stable.push(release("v0.1.0", false, false));
        assert_eq!(picked(with_stable, "0.1.0-alpha.2").as_deref(), Some("0.1.0"));
        assert_eq!(picked(vec![release("v0.2.0-beta.1", true, false), release("v0.1.1", false, false)], "0.1.0").as_deref(), Some("0.1.1"));
    }

    #[test]
    fn sums_are_read_from_sha256sum_listings() {
        let sums = "ab12  gameviber_0.1.0~alpha.1-1_amd64.deb\nCD34 *gameviber-0.1.0~alpha.1-1.x86_64.rpm\n";
        assert_eq!(expected_sum(sums, "gameviber_0.1.0~alpha.1-1_amd64.deb").as_deref(), Some("ab12"));
        assert_eq!(expected_sum(sums, "gameviber-0.1.0~alpha.1-1.x86_64.rpm").as_deref(), Some("cd34"));
        assert_eq!(expected_sum(sums, "gameviber-0.1.0.alpha.1-1.x86_64.rpm").as_deref(), Some("cd34"), "GitHub's name for it");
        assert_eq!(expected_sum(sums, "other.deb"), None);
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    type Files = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    /// Serves the files over HTTP on a local port, for as long as the test runs.
    fn serve(files: Files) -> String {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut stream in listener.incoming().map_while(Result::ok) {
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                // The headers, up to the empty line.
                while reader.read_line(&mut String::new()).is_ok_and(|n| n > 2) {}
                let path = line.split_whitespace().nth(1).unwrap_or_default().to_owned();
                let body = files.lock().unwrap().iter().find(|(name, _)| path == format!("/{name}")).map(|(_, b)| b.clone());
                let (status, body) = match body {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", Vec::new()),
                };
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(&body);
            }
        });
        address
    }

    /// The whole update of an archive installation, against a server answering
    /// like GitHub: found, downloaded, checked, installed; refused when damaged.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_archive_installation_updates_itself() {
        let root = std::env::temp_dir().join(format!("gameviber-update-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let installed = root.join("installed");
        let build = root.join("build");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(build.join("gameviber-9.0.0-x86_64")).unwrap();
        std::fs::write(installed.join("gameviber"), "old").unwrap();
        std::fs::write(build.join("gameviber-9.0.0-x86_64/gameviber"), "new").unwrap();
        let name = "gameviber-9.0.0-x86_64.tar.gz";
        let status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(build.join(name))
            .arg("-C")
            .arg(&build)
            .arg("gameviber-9.0.0-x86_64")
            .status()
            .unwrap();
        assert!(status.success());
        let archive = std::fs::read(build.join(name)).unwrap();

        let files: Files = Arc::default();
        let base = serve(files.clone());
        let publish = |sum: &str| {
            let sums = format!("{sum}  {name}\n");
            let json = serde_json::json!([{
                "tag_name": "v9.0.0", "draft": false, "prerelease": false, "body": "Notes", "html_url": "https://example.com",
                "assets": [
                    {"name": name, "browser_download_url": format!("{base}/{name}"), "size": archive.len()},
                    {"name": SUMS, "browser_download_url": format!("{base}/{SUMS}"), "size": sums.len()},
                ],
            }]);
            *files.lock().unwrap() =
                vec![("releases".into(), json.to_string().into_bytes()), (name.into(), archive.clone()), (SUMS.into(), sums.into_bytes())];
        };
        // SAFETY: no other test reads this variable.
        std::env::set_var("GAMEVIBER_RELEASES_URL", format!("{base}/releases"));
        let installation = Installation::Archive(installed.clone());
        let status = Mutex::new(Status::default());

        publish(&"0".repeat(64));
        let release = newest_release().unwrap().expect("9.0.0 is newer");
        assert_eq!((release.version.to_string(), release.notes.as_str()), ("9.0.0".to_owned(), "Notes"));
        let downloads = root.join("downloads");
        let damaged = install(&status, &installation, &release, &downloads).unwrap_err().to_string();
        assert!(damaged.contains("damaged"), "{damaged}");
        assert_eq!(std::fs::read_to_string(installed.join("gameviber")).unwrap(), "old");

        publish(&hex(&Sha256::digest(&archive)));
        let release = newest_release().unwrap().unwrap();
        install(&status, &installation, &release, &downloads).unwrap();
        assert_eq!(std::fs::read_to_string(installed.join("gameviber")).unwrap(), "new");
        assert!(matches!(status.lock().unwrap().phase, Phase::Installing(_)));
        std::env::remove_var("GAMEVIBER_RELEASES_URL");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn each_installation_gets_its_own_file() {
        assert_eq!(Installation::Package(PackageKind::Arch).asset_suffix(), Some(".pkg.tar.zst"));
        assert_eq!(Installation::Archive(PathBuf::from("/x")).asset_suffix(), Some(".tar.gz"));
        assert_eq!(Installation::Managed("Flathub".into()).asset_suffix(), None);
        assert!(!Installation::Source.installs_updates());
    }
}
