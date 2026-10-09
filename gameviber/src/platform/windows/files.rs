//! File dialogs: the system's own (`IFileOpenDialog`, `IFileSaveDialog`),
//! over GameViber's window. They block until the player picks or cancels:
//! call them from a thread of their own.

use std::path::PathBuf;

use anyhow::Context;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
    FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, SIGDN_FILESYSPATH,
};

/// Asks for a file to open, among the files named `*.<extension>` (`kind`
/// names them in the dialog). None: cancelled.
pub fn open_file(title: &str, kind: &str, extension: &str) -> anyhow::Result<Option<PathBuf>> {
    Ok(open(title, kind, &[extension], false)?.into_iter().next())
}

/// Asks for files to open, among those named after one of `extensions`. Empty: cancelled.
pub fn open_files(title: &str, kind: &str, extensions: &[&str]) -> anyhow::Result<Vec<PathBuf>> {
    open(title, kind, extensions, true)
}

/// Asks where to save a file, suggesting `name`. None: cancelled.
pub fn save_file(title: &str, kind: &str, extension: &str, name: &str) -> anyhow::Result<Option<PathBuf>> {
    with_com(|| unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).context("cannot open the file dialog")?;
        let filter = Filter::new(kind, &[extension]);
        prepare(&dialog, title, &filter)?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)?;
        dialog.SetFileName(&HSTRING::from(name))?;
        dialog.SetDefaultExtension(&HSTRING::from(extension))?;
        if !show(&dialog)? {
            return Ok(None);
        }
        Ok(Some(path_of(&dialog.GetResult()?)?))
    })
}

fn open(title: &str, kind: &str, extensions: &[&str], multiple: bool) -> anyhow::Result<Vec<PathBuf>> {
    with_com(|| unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).context("cannot open the file dialog")?;
        let filter = Filter::new(kind, extensions);
        prepare(&dialog, title, &filter)?;
        let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST;
        if multiple {
            options |= FOS_ALLOWMULTISELECT;
        }
        dialog.SetOptions(options)?;
        if !show(&dialog)? {
            return Ok(Vec::new());
        }
        let items = dialog.GetResults()?;
        (0..items.GetCount()?).map(|i| path_of(&items.GetItemAt(i)?)).collect()
    })
}

/// The dialog's filter: "Kind (*.a;*.b)", kept alive while the dialog shows.
struct Filter {
    name: HSTRING,
    pattern: HSTRING,
}

impl Filter {
    fn new(kind: &str, extensions: &[&str]) -> Self {
        let pattern = extensions.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";");
        Self { name: HSTRING::from(format!("{kind} ({pattern})")), pattern: HSTRING::from(pattern) }
    }
}

unsafe fn prepare(dialog: &IFileDialog, title: &str, filter: &Filter) -> anyhow::Result<()> {
    dialog.SetTitle(&HSTRING::from(title))?;
    let spec = [COMDLG_FILTERSPEC { pszName: PCWSTR(filter.name.as_ptr()), pszSpec: PCWSTR(filter.pattern.as_ptr()) }];
    dialog.SetFileTypes(&spec)?;
    Ok(())
}

/// Shows the dialog over GameViber's window; false: cancelled.
unsafe fn show(dialog: &IFileDialog) -> anyhow::Result<bool> {
    let owner = super::window().map(|w| HWND(w as _));
    match dialog.Show(owner) {
        Ok(()) => Ok(true),
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Ok(false),
        Err(e) => Err(e).context("the file dialog failed"),
    }
}

unsafe fn path_of(item: &IShellItem) -> anyhow::Result<PathBuf> {
    let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
    let path = name.to_string();
    CoTaskMemFree(Some(name.0 as _));
    Ok(PathBuf::from(path?))
}

/// Runs `f` with COM set up on this thread.
fn with_com<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    // SAFETY: balanced by CoUninitialize when it succeeded (also when COM was already set up).
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = f();
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}
