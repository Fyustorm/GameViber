//! Windows: how GameViber was installed comes from a `distribution` file next
//! to `gameviber.exe` (`packaging/windows/`): "installer" (installed by our
//! installer, which installs the update when GameViber closes, then starts it
//! again) or "archive" (our zip archive, whose files are replaced in place).
//! Any other value names the store that updates it.
//!
//! A running executable cannot be overwritten, only renamed: the archive's
//! old files are moved aside as `<name>.old`, and removed the next time.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use anyhow::Context;

use super::{Installation, PackageKind};

const MARKER: &str = "distribution";
/// The release file of the archive (`packaging/windows/package.sh`).
pub const ARCHIVE_SUFFIX: &str = "-windows-x86_64.zip";
const EXE: &str = "gameviber.exe";
/// Old files moved aside end with this.
const OLD: &str = ".old";

/// The installer downloaded, run when GameViber closes.
static INSTALLER: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn installation() -> Installation {
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_owned)) else { return Installation::Source };
    remove_old(&dir);
    match std::fs::read_to_string(dir.join(MARKER)) {
        Ok(marker) => from_marker(marker.trim(), &dir),
        Err(_) => Installation::Source,
    }
}

fn from_marker(marker: &str, dir: &Path) -> Installation {
    match marker {
        "installer" => Installation::Package(PackageKind::Installer),
        "archive" => Installation::Archive(dir.to_owned()),
        other => Installation::Managed(other.to_owned()),
    }
}

/// Installs the downloaded release file (the installer: when GameViber closes).
pub fn apply(installation: &Installation, file: &Path) -> anyhow::Result<()> {
    match installation {
        Installation::Package(PackageKind::Installer) => {
            // The download directory is emptied once this returns.
            let kept = std::env::temp_dir().join(file.file_name().context("no file name")?);
            std::fs::copy(file, &kept).with_context(|| format!("cannot keep {}", kept.display()))?;
            *INSTALLER.lock().unwrap() = Some(kept);
            Ok(())
        }
        Installation::Archive(dir) => replace_archive(dir, file),
        _ => anyhow::bail!("this installation is updated by other means"),
    }
}

/// Extracts the archive next to the installed files, then puts each new file
/// in place of the old one, which is moved aside.
fn replace_archive(dir: &Path, file: &Path) -> anyhow::Result<()> {
    let staging = dir.join(format!(".update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging).with_context(|| format!("cannot write in {}", dir.display()))?;
    let result = (|| {
        extract(file, &staging)?;
        // The archive holds one directory, gameviber-<version>-windows-x86_64.
        let top = std::fs::read_dir(&staging)?.filter_map(Result::ok).find(|e| e.path().is_dir()).context("unexpected archive")?.path();
        anyhow::ensure!(top.join(EXE).is_file(), "unexpected archive: no {EXE} in it");
        move_files(&top, dir)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn extract(file: &Path, to: &Path) -> anyhow::Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(file)?).context("cannot read the archive")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // Paths leaving the directory are refused.
        let Some(relative) = entry.enclosed_name() else { anyhow::bail!("unexpected archive: {}", entry.name()) };
        let path = to.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        std::fs::write(&path, bytes)?;
    }
    Ok(())
}

fn move_files(from: &Path, to: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(from)?.collect::<Result<Vec<_>, _>>()? {
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            move_files(&entry.path(), &target)?;
            continue;
        }
        if target.exists() {
            let aside = old_name(&target);
            let _ = std::fs::remove_file(&aside);
            std::fs::rename(&target, &aside).with_context(|| format!("cannot replace {}", target.display()))?;
        }
        std::fs::rename(entry.path(), &target).with_context(|| format!("cannot replace {}", target.display()))?;
    }
    Ok(())
}

fn old_name(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(OLD);
    PathBuf::from(name)
}

/// Removes the files a previous update moved aside.
fn remove_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            remove_old(&path);
        } else if path.to_string_lossy().ends_with(OLD) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Starts the installer of the update (which starts GameViber again once
/// done), or the updated GameViber, with the same arguments; then quits.
pub fn relaunch() -> anyhow::Error {
    if let Some(installer) = INSTALLER.lock().unwrap().take() {
        log::info!("installing the update: {}", installer.display());
        // Windows asks the user to allow it (it installs for every user).
        return match Command::new(&installer).args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"]).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => anyhow::Error::new(e).context(installer.display().to_string()),
        };
    }
    let Ok(exe) = std::env::current_exe() else { return anyhow::anyhow!("this executable is unknown") };
    log::info!("starting GameViber again: {}", exe.display());
    match Command::new(&exe).args(std::env::args_os().skip(1)).spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => anyhow::Error::new(e).context(exe.display().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_files_carry_their_marker() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packaging/windows");
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(read("package.sh").contains(&format!("printf 'archive\\r\\n' > \"$work/$name/{MARKER}\"")));
        // The archive and the installer (`Installation::asset_suffix`) are named after `name`.
        assert!(read("package.sh").contains(&format!("name=gameviber-$version{}\n", ARCHIVE_SUFFIX.trim_end_matches(".zip"))));
        assert!(read("package.sh").contains("/$name.zip\"") && read("package.sh").contains("-DOutputName=$name-setup"));
        assert!(read("gameviber.iss").contains(&format!("{MARKER}-installer\"; DestDir: \"{{app}}\"; DestName: \"{MARKER}\"")));
    }

    #[test]
    fn markers_name_the_installation() {
        let dir = Path::new(r"C:\GameViber");
        assert_eq!(from_marker("installer", dir), Installation::Package(PackageKind::Installer));
        assert_eq!(from_marker("archive", dir), Installation::Archive(dir.to_owned()));
        assert_eq!(from_marker("Microsoft Store", dir), Installation::Managed("Microsoft Store".into()));
    }

    #[test]
    fn an_archive_update_replaces_its_files() {
        let root = std::env::temp_dir().join(format!("gameviber-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let installed = root.join("installed");
        std::fs::create_dir_all(installed.join("models")).unwrap();
        std::fs::write(installed.join(EXE), "old").unwrap();
        std::fs::write(installed.join("models/kept"), "mine").unwrap();
        let archive = root.join("update.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (name, text) in [("gameviber-9.9.9-windows-x86_64/gameviber.exe", "new"), ("gameviber-9.9.9-windows-x86_64/licenses/LICENSE", "GPL")] {
                zip.start_file(name, options).unwrap();
                std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }

        replace_archive(&installed, &archive).unwrap();
        let read = |p: &str| std::fs::read_to_string(installed.join(p)).unwrap();
        assert_eq!(read(EXE), "new");
        assert_eq!(read("licenses/LICENSE"), "GPL");
        assert_eq!(read("models/kept"), "mine");
        assert_eq!(read("gameviber.exe.old"), "old", "moved aside");
        remove_old(&installed);
        assert!(!installed.join("gameviber.exe.old").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
