pub mod commands;
pub mod engine;
pub mod error;
pub mod extract;
pub mod store;
pub mod sys;

use std::sync::Arc;

use tauri::Manager as _;

use crate::store::{Settings, Store};

pub fn run() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_target(false).init();

    let portable = sys::portable::data_dir();
    if let Some(dir) = &portable {
        // WebView2 reads this when Tauri creates the window; it keeps the browser profile inside the folder too.
        // SAFETY: called on the main thread before Tauri or anything else starts other threads.
        unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir.join("webview")) };
        tracing::info!(dir = %dir.display(), "portable mode");
    }

    let mut builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }));
    // The window-state plugin writes to AppData; a portable copy simply opens centred.
    if portable.is_none() {
        builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(move |app| {
            let data = match &portable {
                Some(dir) => dir.clone(),
                None => app.path().app_data_dir()?,
            };
            let store = Arc::new(Store::open(&data.join("rabbit.db"))?);
            let settings = match store.load_settings()? {
                Some(s) => s,
                None => {
                    let s = Settings { download_dir: app.path().download_dir()?.join("Rabbit"), max_concurrent: 3, max_segments: 16, notifications: true, ytdlp_channel: Default::default(), sessions: Default::default() };
                    store.save_settings(&s)?;
                    s
                }
            };
            let tools = sys::tools::Tools::discover_in(Some(&app.path().resource_dir()?));
            tracing::info!(?tools, "tools discovered");
            let manager = engine::manager::Manager::new(store.clone(), settings, tools)?;
            manager.attach_app(app.handle().clone());
            // Verified yt-dlp updates live in the per-user data folder (Program Files isn't writable).
            let update_root = match &portable {
                Some(dir) => dir.clone(),
                None => app.path().app_local_data_dir()?,
            };
            manager.set_update_dir(update_root.join("bin"));
            app.manage(store);
            app.manage(manager);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::subscribe,
            commands::detect_input,
            commands::analyze,
            commands::cancel_analyze,
            commands::enqueue,
            commands::list_tasks,
            commands::cancel_task,
            commands::remove_task,
            commands::pause_task,
            commands::resume_task,
            commands::pause_all,
            commands::resume_all,
            commands::open_task,
            commands::library_list,
            commands::library_facets,
            commands::library_open,
            commands::library_remove,
            commands::get_settings,
            commands::set_settings,
            commands::choose_download_dir,
            commands::tool_status,
            commands::ytdlp_status,
            commands::ytdlp_check,
            commands::ytdlp_install,
            commands::ytdlp_use_bundled,
            commands::list_firefox_profiles,
            commands::choose_cookie_file,
            commands::session_check,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rabbit");
}
