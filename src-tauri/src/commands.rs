use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::engine::manager::{detect_input as detect, DetectedUrl, Manager, SessionStatus, TaskEvent, TaskInfo, UpdateEvent, YtDlpStatus};
use crate::sys::cookies::{self, FirefoxProfile, Site};
use crate::sys::updater::{Channel as UpdateChannel, UpdateCheck};
use crate::error::{AppError, AppResult};
use crate::extract::model::MediaInfo;
use crate::store::{LibraryFacets, LibraryItem, LibraryQuery, Settings, Store};
use crate::sys::tools::Tools;

type M<'a> = State<'a, Arc<Manager>>;

#[tauri::command]
pub fn subscribe(m: M<'_>, channel: Channel<TaskEvent>) -> Vec<TaskInfo> {
    m.subscribe(channel)
}

#[tauri::command]
pub fn detect_input(text: String) -> Vec<DetectedUrl> {
    detect(&text)
}

#[tauri::command]
pub async fn analyze(m: M<'_>, input: String) -> AppResult<MediaInfo> {
    m.analyze(&input).await
}

#[tauri::command]
pub fn cancel_analyze(m: M<'_>) {
    m.cancel_analyze();
}

#[tauri::command]
pub fn enqueue(m: M<'_>, analysis_id: String, option_id: String) -> AppResult<TaskInfo> {
    m.inner().enqueue(&analysis_id, &option_id)
}

#[tauri::command]
pub fn list_tasks(m: M<'_>) -> Vec<TaskInfo> {
    m.list()
}

#[tauri::command]
pub fn cancel_task(m: M<'_>, id: String) -> AppResult<()> {
    m.cancel(&id)
}

#[tauri::command]
pub fn remove_task(m: M<'_>, id: String) -> AppResult<()> {
    m.remove(&id)
}

#[tauri::command]
pub fn pause_task(m: M<'_>, id: String) -> AppResult<()> {
    m.pause(&id)
}

/// Resumes a paused task, or retries a failed or cancelled one.
#[tauri::command]
pub fn resume_task(m: M<'_>, id: String) -> AppResult<()> {
    m.inner().resume(&id)
}

#[tauri::command]
pub fn pause_all(m: M<'_>) {
    m.pause_all();
}

#[tauri::command]
pub fn resume_all(m: M<'_>) {
    m.inner().resume_all();
}

fn open(app: &AppHandle, path: PathBuf, reveal: bool) -> AppResult<()> {
    if !path.exists() {
        return Err(AppError::NotFound("The file was moved or deleted.".into()));
    }
    let r = if reveal { app.opener().reveal_item_in_dir(&path) } else { app.opener().open_path(path.to_string_lossy(), None::<&str>) };
    r.map_err(|e| AppError::Tool(format!("Couldn't open it: {e}")))
}

#[tauri::command]
pub fn open_task(app: AppHandle, m: M<'_>, id: String, reveal: bool) -> AppResult<()> {
    open(&app, m.task_path(&id)?, reveal)
}

#[tauri::command]
pub fn library_list(store: State<'_, Arc<Store>>, query: LibraryQuery) -> AppResult<Vec<LibraryItem>> {
    store.list(&query, 200)
}

#[tauri::command]
pub fn library_facets(store: State<'_, Arc<Store>>) -> AppResult<LibraryFacets> {
    store.facets()
}

#[tauri::command]
pub fn library_open(app: AppHandle, store: State<'_, Arc<Store>>, id: i64, reveal: bool) -> AppResult<()> {
    open(&app, store.get_path(id)?, reveal)
}

#[tauri::command]
pub fn library_remove(store: State<'_, Arc<Store>>, id: i64, delete_file: bool) -> AppResult<()> {
    if delete_file {
        let p = store.get_path(id)?;
        if p.is_dir() {
            std::fs::remove_dir_all(&p)?;
        } else if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    store.remove(id)
}

#[tauri::command]
pub fn get_settings(m: M<'_>) -> Settings {
    m.settings()
}

#[tauri::command]
pub fn set_settings(m: M<'_>, settings: Settings) -> AppResult<Settings> {
    m.update_settings(settings)
}

#[tauri::command]
pub async fn choose_download_dir(app: AppHandle, m: M<'_>) -> AppResult<Option<Settings>> {
    let current = m.settings();
    let picked = app.dialog().file().set_directory(&current.download_dir).blocking_pick_folder();
    let Some(p) = picked.and_then(|f| f.into_path().ok()) else { return Ok(None) };
    m.update_settings(Settings { download_dir: p, ..current }).map(Some)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    tools: Tools,
    version: &'static str,
}

#[tauri::command]
pub fn tool_status(m: M<'_>) -> ToolStatus {
    ToolStatus { tools: m.tools(), version: env!("CARGO_PKG_VERSION") }
}

#[tauri::command]
pub async fn ytdlp_status(m: M<'_>) -> AppResult<YtDlpStatus> {
    Ok(m.ytdlp_status().await)
}

/// Contacts GitHub only when the user presses "Check for updates".
#[tauri::command]
pub async fn ytdlp_check(m: M<'_>, channel: UpdateChannel) -> AppResult<UpdateCheck> {
    m.ytdlp_check(channel).await
}

#[tauri::command]
pub async fn ytdlp_install(m: M<'_>, channel: UpdateChannel, tag: String, on_progress: tauri::ipc::Channel<UpdateEvent>) -> AppResult<YtDlpStatus> {
    m.ytdlp_install(channel, &tag, Some(on_progress)).await
}

#[tauri::command]
pub async fn ytdlp_use_bundled(m: M<'_>) -> AppResult<YtDlpStatus> {
    m.ytdlp_use_bundled().await
}

/// Firefox profiles found on this PC, for Settings → Signed-in sites.
#[tauri::command]
pub fn list_firefox_profiles() -> Vec<FirefoxProfile> {
    cookies::firefox_profiles()
}

/// Lets the user pick an exported cookies.txt. Only the path is returned (and later stored).
#[tauri::command]
pub async fn choose_cookie_file(app: AppHandle) -> Option<String> {
    let picked = app.dialog().file().add_filter("cookies.txt", &["txt"]).blocking_pick_file();
    picked.and_then(|f| f.into_path().ok()).map(|p| p.display().to_string())
}

/// Reads the chosen source and asks the site whether the session is signed in.
#[tauri::command]
pub async fn session_check(m: M<'_>, site: Site) -> AppResult<SessionStatus> {
    m.session_check(site).await
}