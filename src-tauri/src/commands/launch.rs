//! Command-line launches (`1-1 import --name … --host …`).
//!
//! The arguments of this launch wait until the window asks for them. A later
//! launch while 1-1 runs is handed to the running window by the single-instance
//! plugin, which brings the window forward. The window parses and confirms the
//! import itself, so nothing is saved without the user seeing it.

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

/// The event a running window receives with a later launch's arguments.
pub(crate) const LAUNCH_ARGS_EVENT: &str = "launch-args";

/// This launch's arguments after the program name, until the window takes them.
#[derive(Default)]
pub(crate) struct LaunchArgs(Mutex<Option<Vec<String>>>);

impl LaunchArgs {
    pub(crate) fn from_env() -> Self {
        Self(Mutex::new(arguments(std::env::args())))
    }
}

/// The arguments after the program name, or `None` when there are none.
fn arguments(argv: impl IntoIterator<Item = String>) -> Option<Vec<String>> {
    let args: Vec<String> = argv.into_iter().skip(1).collect();
    (!args.is_empty()).then_some(args)
}

#[tauri::command]
pub(crate) fn take_launch_args(state: tauri::State<'_, LaunchArgs>) -> Option<Vec<String>> {
    state.0.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// A second launch: show the running window and hand it the arguments.
pub(crate) fn forward(app: &AppHandle, argv: Vec<String>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(args) = arguments(argv)
        && let Err(error) = app.emit(LAUNCH_ARGS_EVENT, args)
    {
        log::warn!("Could not hand the command line to the window: {error}");
    }
}

/// Put text on the local clipboard, such as a copied command line. Clipboard
/// access runs off the UI thread (required on Linux).
#[tauri::command]
pub(crate) async fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    tokio::task::spawn_blocking(move || app.clipboard().write_text(text))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not copy to the clipboard: {e}"))
}

#[cfg(test)]
mod tests {
    use super::arguments;

    #[test]
    fn only_arguments_after_the_program_are_kept() {
        let argv = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(arguments(argv(&["1-1.exe"])), None);
        assert_eq!(
            arguments(argv(&["1-1.exe", "import", "--host", "box"])),
            Some(argv(&["import", "--host", "box"]))
        );
    }
}
