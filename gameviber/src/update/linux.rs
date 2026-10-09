//! Linux: how GameViber was installed comes from a `distribution` file our
//! packages and archive ship (`packaging/linux/`): "package" (installed in
//! /usr/lib/gameviber, updated through the package manager that owns it,
//! with pkexec) or "archive" (next to the executable, whose files are replaced
//! in place). Any other value names the store or repository that updates it.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;

use super::{Installation, PackageKind};
use crate::platform::linux::helper;

const MARKER: &str = "distribution";
/// The release file of the archive (`packaging/linux/package.sh`).
pub const ARCHIVE_SUFFIX: &str = ".tar.gz";
const PACKAGE_EXE: &str = "/usr/bin/gameviber";
const PACKAGE_DIR: &str = "/usr/lib/gameviber";

/// This executable. After a package update, the kernel shows the replaced
/// file as "<path> (deleted)".
fn executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let text = exe.to_string_lossy();
    Some(text.strip_suffix(" (deleted)").map(PathBuf::from).unwrap_or(exe))
}

pub fn installation() -> Installation {
    if std::env::var_os("FLATPAK_ID").is_some() {
        return Installation::Managed("Flatpak".into());
    }
    let Some(exe) = executable() else { return Installation::Source };
    let Some(dir) = exe.parent() else { return Installation::Source };
    if let Ok(marker) = std::fs::read_to_string(dir.join(MARKER)) {
        return from_marker(marker.trim(), dir);
    }
    if exe == Path::new(PACKAGE_EXE) {
        if let Ok(marker) = std::fs::read_to_string(Path::new(PACKAGE_DIR).join(MARKER)) {
            return from_marker(marker.trim(), dir);
        }
    }
    Installation::Source
}

fn from_marker(marker: &str, dir: &Path) -> Installation {
    match marker {
        "archive" => Installation::Archive(dir.to_owned()),
        "package" => match package_manager() {
            Some(kind) => Installation::Package(kind),
            None => Installation::Managed("your package manager".into()),
        },
        other => Installation::Managed(other.to_owned()),
    }
}

/// The package manager that installed /usr/bin/gameviber.
fn package_manager() -> Option<PackageKind> {
    let owns = |program: &str, args: &[&str]| {
        Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    };
    if owns("dpkg-query", &["-S", PACKAGE_EXE]) {
        Some(PackageKind::Deb)
    } else if owns("pacman", &["-Qo", PACKAGE_EXE]) {
        Some(PackageKind::Arch)
    } else if owns("rpm", &["-qf", PACKAGE_EXE]) {
        Some(PackageKind::Rpm)
    } else {
        None
    }
}

/// Installs the downloaded release file.
pub fn apply(installation: &Installation, file: &Path) -> anyhow::Result<()> {
    match installation {
        Installation::Package(kind) => install_package(*kind, file),
        Installation::Archive(dir) => replace_archive(dir, file),
        Installation::Managed(_) | Installation::Source => anyhow::bail!("this installation is updated by other means"),
    }
}

fn exists(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// Through the package manager, as root: pkexec asks for the password.
fn install_package(kind: PackageKind, file: &Path) -> anyhow::Result<()> {
    let file = file.to_string_lossy().into_owned();
    let command: Vec<&str> = match kind {
        PackageKind::Deb => vec!["apt-get", "install", "-y", &file],
        PackageKind::Rpm if exists("dnf") => vec!["dnf", "install", "-y", &file],
        PackageKind::Rpm if exists("zypper") => vec!["zypper", "--non-interactive", "install", "--allow-unsigned-rpm", &file],
        PackageKind::Rpm => vec!["rpm", "-U", &file],
        PackageKind::Arch => vec!["pacman", "-U", "--noconfirm", &file],
        PackageKind::Installer => anyhow::bail!("not a Linux package"),
    };
    log::info!("installing the update: pkexec {}", command.join(" "));
    helper::dialog::prepare(helper::PKEXEC_ACTION);
    let output = Command::new("pkexec").args(&command).stdin(Stdio::null()).output().context("cannot start pkexec")?;
    match output.status.code() {
        Some(0) => Ok(()),
        // pkexec: the authorization was refused or the dialog closed.
        Some(126 | 127) => anyhow::bail!("authorization refused or cancelled"),
        _ => {
            let errors = String::from_utf8_lossy(&output.stderr);
            let last: Vec<&str> = errors.lines().filter(|l| !l.trim().is_empty()).collect();
            anyhow::bail!("{} failed: {}", command[0], last[last.len().saturating_sub(3)..].join(" / "))
        }
    }
}

/// Extracts the archive next to the installed files, then moves each new
/// file over the old one (a rename: the running GameViber keeps its own).
fn replace_archive(dir: &Path, file: &Path) -> anyhow::Result<()> {
    let staging = dir.join(format!(".update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging).with_context(|| format!("cannot write in {}", dir.display()))?;
    let result = (|| {
        let status = Command::new("tar").arg("-xzf").arg(file).arg("-C").arg(&staging).status().context("cannot start tar")?;
        anyhow::ensure!(status.success(), "cannot extract the archive");
        // The archive holds one directory, gameviber-<version>-x86_64.
        let top = std::fs::read_dir(&staging)?.filter_map(Result::ok).find(|e| e.path().is_dir()).context("unexpected archive")?.path();
        anyhow::ensure!(top.join("gameviber").is_file(), "unexpected archive: no gameviber in it");
        move_files(&top, dir)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn move_files(from: &Path, to: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            move_files(&entry.path(), &target)?;
        } else {
            std::fs::rename(entry.path(), &target).with_context(|| format!("cannot replace {}", target.display()))?;
        }
    }
    Ok(())
}

/// Replaces this process with the installed GameViber, with the same arguments.
pub fn relaunch() -> anyhow::Error {
    let Some(exe) = executable() else { return anyhow::anyhow!("this executable is unknown") };
    log::info!("starting GameViber again: {}", exe.display());
    let error = Command::new(&exe).args(std::env::args_os().skip(1)).exec();
    anyhow::Error::new(error).context(exe.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_files_carry_their_marker() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packaging/linux");
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(read("stage.sh").contains(&format!("printf 'package\\n' > \"$dest{PACKAGE_DIR}/{MARKER}\"")));
        assert!(read("nfpm.yaml").contains(&format!("dst: {PACKAGE_DIR}/{MARKER}")));
        assert!(read("package.sh").contains(&format!("printf 'archive\\n' > \"$out/$name/{MARKER}\"")));
    }

    #[test]
    fn markers_name_the_installation() {
        let dir = Path::new("/opt/gameviber");
        assert_eq!(from_marker("archive", dir), Installation::Archive(dir.to_owned()));
        assert_eq!(from_marker("Flathub", dir), Installation::Managed("Flathub".into()));
    }

    #[test]
    fn an_archive_update_replaces_its_files() {
        let root = std::env::temp_dir().join(format!("gameviber-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let installed = root.join("installed");
        let new = root.join("gameviber-9.9.9-x86_64");
        std::fs::create_dir_all(installed.join("lib32")).unwrap();
        std::fs::create_dir_all(new.join("lib32")).unwrap();
        std::fs::write(installed.join("gameviber"), "old").unwrap();
        std::fs::write(installed.join("lib32/libgameviber_overlay.so"), "old").unwrap();
        std::fs::write(installed.join("settings-kept"), "mine").unwrap();
        std::fs::write(new.join("gameviber"), "new").unwrap();
        std::fs::write(new.join("lib32/libgameviber_overlay.so"), "new").unwrap();
        let archive = root.join("update.tar.gz");
        let tar = Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&root).arg("gameviber-9.9.9-x86_64").status().unwrap();
        assert!(tar.success());

        replace_archive(&installed, &archive).unwrap();
        let read = |p: &str| std::fs::read_to_string(installed.join(p)).unwrap();
        assert_eq!(read("gameviber"), "new");
        assert_eq!(read("lib32/libgameviber_overlay.so"), "new");
        assert_eq!(read("settings-kept"), "mine");
        assert_eq!(std::fs::read_dir(&installed).unwrap().count(), 3, "no staging directory left");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
