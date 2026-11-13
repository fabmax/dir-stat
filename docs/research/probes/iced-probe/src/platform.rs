//! rfd / opener / trash probes, wrapped as iced Tasks.
use iced::Task;

use std::path::{Path, PathBuf};

/// Folder picker. IMPORTANT (macOS): `pick_folder()` builds the NSOpenPanel *eagerly when it is
/// called*, not when the future is first polled. Calling it here (inside `update`, i.e. on the
/// main thread) and handing the returned future to Task::perform is the pattern rfd recommends
/// ("spawn on main, await elsewhere"). The panel is shown as a sheet attached to the iced window.
pub fn pick_folder<Message>(
    start: Option<PathBuf>,
    on_picked: fn(Option<PathBuf>) -> Message,
) -> Task<Message>
where
    Message: Send + 'static,
{
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("Choose a folder to analyze")
        .set_can_create_directories(false);

    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }

    // Future type: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>>
    let future = dialog.pick_folder();

    Task::perform(future, move |handle| {
        on_picked(handle.map(|handle| handle.path().to_path_buf()))
    })
}

/// The same thing written as an `async` block (also compiles; on macOS the panel is then
/// created on a tokio worker thread and rfd hops to the main thread via GCD `exec_sync`).
#[allow(dead_code)]
pub fn pick_folder_async_block<Message>(
    start: PathBuf,
    on_picked: fn(Option<PathBuf>) -> Message,
) -> Task<Message>
where
    Message: Send + 'static,
{
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .set_directory(start)
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
        on_picked,
    )
}

/// "Show in Finder" / file manager. Needs `opener = { version = "0.9", features = ["reveal"] }`.
/// opener runs `open -R <path>` on macOS and waits for it, so keep it off the UI thread.
pub fn reveal<Message>(
    path: PathBuf,
    on_done: fn(Result<(), String>) -> Message,
) -> Task<Message>
where
    Message: Send + 'static,
{
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || opener::reveal(&path).map_err(|e| e.to_string()))
                .await
                .map_err(|e| e.to_string())?
        },
        on_done,
    )
}

/// Open with the default application (`open <path>` on macOS, xdg-open on Linux).
pub fn open<Message>(
    path: PathBuf,
    on_done: fn(Result<(), String>) -> Message,
) -> Task<Message>
where
    Message: Send + 'static,
{
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || opener::open(&path).map_err(|e| e.to_string()))
                .await
                .map_err(|e| e.to_string())?
        },
        on_done,
    )
}

/// Move to Trash. Never called automatically by the probe.
pub fn move_to_trash<Message>(
    path: PathBuf,
    on_done: fn(Result<PathBuf, String>) -> Message,
) -> Task<Message>
where
    Message: Send + 'static,
{
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                trash_path(&path).map(|()| path).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?
        },
        on_done,
    )
}

/// macOS: the default trash::DeleteMethod::Finder runs `osascript ... tell application "Finder"`
/// which triggers an Automation (Apple Events) permission prompt and plays the Finder sound.
/// NsFileManager uses -[NSFileManager trashItemAtURL:] instead: no prompt, faster, but no
/// "Put Back" in Finder on some systems.
fn trash_path(path: &Path) -> Result<(), trash::Error> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};

        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context.delete(path)
    }

    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path)
    }
}
