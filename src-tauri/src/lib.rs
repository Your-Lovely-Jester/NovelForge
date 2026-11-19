mod books;
mod cache;
mod catalogue;
mod chapter_clean;
mod commands;
mod duplicates;
mod error;
pub mod export;
mod fsutil;
mod library_actions;
mod metadata;
pub mod model;
mod paths;
mod process;
#[cfg(debug_assertions)]
mod qa;
mod resolver;
mod settings;
mod sources;
mod spawn;
mod storage;
mod structurer;
mod timeutil;
mod tools;
mod transport;
mod verify;

use commands::{
    adopt_archive, cancel_download, check_tools, cleanup_engine_work, clear_finished,
    clear_hardcover_token, confirm_quit, delete_download, delete_partial_downloads,
    fetch_novel_chapters, fetch_novel_metadata, find_existing_download, get_app_info,
    get_diagnostics, get_download_details, get_hardcover_status, get_settings, get_tasks,
    get_transport_report, list_library, list_manga_sources, list_source_catalogue,
    list_source_profiles, open_chapters, open_donation, open_downloads, open_file, pause_download,
    probe_tool, repack_manga, resolve_title, resume_download, resume_interrupted, retry_download,
    reveal_path, save_settings, search_books, search_webnovels, set_hardcover_token,
    start_download, suggest_faster_source, update_download, verify_path,
};
use model::EVENT_QUIT_REQUESTED;
use process::AppState;
use settings::Settings;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch must raise the running window instead of opening
            // a second copy that would race the settings and job journal.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;

            let config_file = config_dir.join("settings.json");
            #[cfg_attr(not(debug_assertions), allow(unused_mut))]
            let mut settings = Settings::load(&config_file);
            #[cfg(debug_assertions)]
            {
                settings = qa::apply_source_limits(settings);
            }
            transport::configure(&settings.source_limits);

            if let Err(error) = settings::prepare_download_dir(&settings) {
                return Err(std::io::Error::other(error.to_string()).into());
            }

            app.manage(AppState::new(settings, config_file));
            app.state::<AppState>().restore_journal();

            #[cfg(debug_assertions)]
            if qa::requested_library() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_library(handle.clone()).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some(action) = qa::requested_storage() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_storage(handle.clone(), action).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some(path) = qa::requested_adopt() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_adopt(handle.clone(), path).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some((action, folder, refresh)) = qa::requested_action() {
                let handle = app.handle().clone();
                let state = app.state::<AppState>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_action(handle.clone(), state, action, folder, refresh).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some((query, sources)) = qa::requested_search() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_search(handle.clone(), query, sources).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some((query, mode, sources)) = qa::requested_resolve() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_resolve(handle.clone(), query, mode, sources).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some(url) = qa::requested_transport() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_transport(url).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some(url) = qa::requested_index() {
                let handle = app.handle().clone();
                let state = app.state::<AppState>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_index(state, url).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if qa::requested_manga_sources() {
                let handle = app.handle().clone();
                let state = app.state::<AppState>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_manga_sources(handle.clone(), state).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some((query, mode)) = qa::requested_book_search() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_book_search(handle.clone(), query, mode).await;
                    handle.exit(0);
                });
            }

            #[cfg(debug_assertions)]
            if let Some((query, chapters, engine)) = qa::requested_query() {
                let handle = app.handle().clone();
                let state = app.state::<AppState>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    qa::run_download(handle.clone(), state, query, chapters, engine).await;
                    handle.exit(0);
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<AppState>();
                if !state.allow_exit.load(Ordering::SeqCst)
                    && process::quit_needs_confirmation(&state)
                {
                    api.prevent_close();
                    let _ = window.app_handle().emit(EVENT_QUIT_REQUESTED, ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            check_tools,
            probe_tool,
            resolve_title,
            search_webnovels,
            search_books,
            get_hardcover_status,
            set_hardcover_token,
            clear_hardcover_token,
            fetch_novel_metadata,
            fetch_novel_chapters,
            find_existing_download,
            get_download_details,
            delete_download,
            retry_download,
            update_download,
            suggest_faster_source,
            open_chapters,
            list_manga_sources,
            start_download,
            pause_download,
            resume_download,
            resume_interrupted,
            cancel_download,
            clear_finished,
            get_tasks,
            get_settings,
            save_settings,
            get_transport_report,
            list_source_profiles,
            list_source_catalogue,
            list_library,
            adopt_archive,
            cleanup_engine_work,
            delete_partial_downloads,
            verify_path,
            reveal_path,
            open_file,
            repack_manga,
            open_downloads,
            open_donation,
            get_app_info,
            confirm_quit,
            get_diagnostics
        ])
        .build(tauri::generate_context!())
        .expect("error while building the NovelForge application");

    app.run(|handle, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            let Some(state) = handle.try_state::<AppState>() else {
                return;
            };
            // A menu quit, Alt-F4 or OS logout goes straight to ExitRequested;
            // hold it while jobs are live so the frontend can ask first.
            if !state.allow_exit.load(Ordering::SeqCst) && process::quit_needs_confirmation(&state)
            {
                api.prevent_exit();
                let _ = handle.emit(EVENT_QUIT_REQUESTED, ());
                return;
            }
            process::shutdown_all(&state);
        }
        tauri::RunEvent::Exit => {
            if let Some(state) = handle.try_state::<AppState>() {
                state.persist_journal();
            }
        }
        _ => {}
    });
}
