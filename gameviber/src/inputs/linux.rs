//! Linux: the named pipe (a FIFO) other programs write messages to.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{parse, reject, Message, Status};

/// `$XDG_RUNTIME_DIR/gameviber/inputs`: `echo '{"event":"kill"}' > $XDG_RUNTIME_DIR/gameviber/inputs`.
fn pipe_path() -> PathBuf {
    crate::platform::linux::runtime_dir().join("gameviber").join("inputs")
}

pub fn start_pipe(tx: mpsc::UnboundedSender<Message>, status: Arc<Mutex<Status>>) {
    let path = pipe_path();
    let made = path.parent().map(std::fs::create_dir_all).transpose().map(|_| ()).and_then(|()| {
        use std::os::unix::fs::FileTypeExt;
        if std::fs::metadata(&path).is_ok_and(|m| m.file_type().is_fifo()) {
            return Ok(());
        }
        let _ = std::fs::remove_file(&path);
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
        // SAFETY: mkfifo with a valid path; only the user may write to it.
        match unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } {
            0 => Ok(()),
            _ => Err(std::io::Error::last_os_error()),
        }
    });
    if let Err(e) = made {
        log::warn!("inputs: no pipe at {}: {e}", path.display());
        return;
    }
    status.lock().unwrap().pipe = Some(path.clone());
    std::thread::Builder::new()
        .name("inputs-pipe".into())
        .spawn(move || loop {
            // Opening blocks until a writer comes; reading ends when the last writer leaves.
            let Ok(file) = std::fs::File::open(&path) else { return };
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                match parse(&line) {
                    Ok(messages) => {
                        for m in messages {
                            if tx.send(m).is_err() {
                                return;
                            }
                        }
                    }
                    Err(why) => reject(&status, &line, why),
                }
            }
        })
        .expect("spawn the inputs pipe thread");
}
