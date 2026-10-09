//! The desktop entry: what the desktop knows GameViber by (the shortcuts
//! portal needs it) and how it opens `gameviber://` links (`links`). A
//! package installs it (`packaging/linux/`); without one, GameViber writes
//! its own, launching this executable.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::links::SCHEME;
use crate::shortcuts::APP_ID;

/// The desktop entry a package installs (`packaging/linux/`).
fn system_entry_path() -> PathBuf {
    Path::new("/usr/share/applications").join(format!("{APP_ID}.desktop"))
}

fn user_entries() -> PathBuf {
    super::data_home().join("applications")
}

/// The start of the entries GameViber writes, up to the executable.
const ENTRY_HEAD: &str = "[Desktop Entry]\nType=Application\nName=GameViber\nComment=Game rumble, sound and image to toys\nExec=";

fn entry_text(exe: &Path) -> String {
    format!("{ENTRY_HEAD}{} %u\nIcon=input-gaming\nCategories=Game;\nMimeType=x-scheme-handler/{SCHEME};\n", exec_quoted(exe))
}

/// The executable as the Exec key wants it: quoted when it holds a space or a
/// reserved character. Inside the quotes `"`, `` ` ``, `$` and `\` take a
/// backslash, itself escaped once more as the key is a string (`\\$`).
fn exec_quoted(exe: &Path) -> String {
    let path = exe.display().to_string();
    if !path.chars().any(|c| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c)) {
        return path;
    }
    let mut quoted = String::from("\"");
    for c in path.chars() {
        match c {
            '\\' => quoted.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// `~/.local/share/applications/<APP_ID>.desktop`, launching this executable,
/// unless a package installed one (then a user entry we wrote earlier, which
/// would hide it, is removed).
pub fn desktop_entry() -> anyhow::Result<()> {
    let path = user_entries().join(format!("{APP_ID}.desktop"));
    if system_entry_path().exists() {
        if std::fs::read_to_string(&path).is_ok_and(|e| e.starts_with(ENTRY_HEAD)) {
            std::fs::remove_file(&path)?;
            log::info!("desktop entry of the package used: {} removed", path.display());
            refresh_user_entries();
        }
        return Ok(());
    }
    let entry = entry_text(&std::env::current_exe()?);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, entry)?;
        log::info!("desktop entry written: {}", path.display());
        refresh_user_entries();
    }
    Ok(())
}

/// The desktop's cache of which entry opens what (packages refresh the system one).
fn refresh_user_entries() {
    let _ = Command::new("update-desktop-database").arg("-q").arg(user_entries()).status();
}

/// Makes GameViber open `gameviber://` links: its entry, and the player's
/// default handler of the scheme when they have none yet. Run in the
/// background: the tools it calls can be slow.
pub fn open_links() {
    let spawned = std::thread::Builder::new().name("desktop-entry".into()).spawn(|| {
        if let Err(e) = desktop_entry() {
            log::warn!("cannot write the desktop entry: {e:#}");
        }
        let scheme = format!("x-scheme-handler/{SCHEME}");
        let Ok(query) = Command::new("xdg-mime").args(["query", "default", &scheme]).output() else {
            return log::info!("no xdg-mime: {SCHEME}:// links are left to the desktop");
        };
        if String::from_utf8_lossy(&query.stdout).trim().is_empty() {
            match Command::new("xdg-mime").args(["default", &format!("{APP_ID}.desktop"), &scheme]).status() {
                Ok(s) if s.success() => log::info!("GameViber opens {SCHEME}:// links"),
                _ => log::warn!("cannot make GameViber open {SCHEME}:// links"),
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("cannot register the desktop entry: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_entry_is_the_one_gameviber_writes() {
        let package = include_str!("../../../../packaging/linux/io.github.gameviber.GameViber.desktop");
        assert_eq!(package, entry_text(Path::new("gameviber")));
        assert!(system_entry_path().ends_with(format!("{APP_ID}.desktop")));
    }

    #[test]
    fn odd_executable_paths_are_quoted() {
        assert_eq!(exec_quoted(Path::new("/opt/gameviber/gameviber")), "/opt/gameviber/gameviber");
        assert_eq!(exec_quoted(Path::new("/home/me/My Games/gameviber")), "\"/home/me/My Games/gameviber\"");
        assert_eq!(exec_quoted(Path::new("/home/me/$x/game\"viber")), r#""/home/me/\\$x/game\\"viber""#);
    }
}
