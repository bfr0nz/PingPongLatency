mod commands;
mod db;
mod models;
mod ping;
mod scheduler;

use std::{path::PathBuf, sync::Arc};
use tauri::Manager;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db_path: Arc<PathBuf>,
    pub scheduler: Arc<Mutex<()>>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let db_path = app.path().app_data_dir()?.join("latency.sqlite3");

            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            db::initialize_database(&db_path)?;

            let state = AppState {
                db_path: Arc::new(db_path),
                scheduler: Arc::new(Mutex::new(())),
            };

            app.manage(state.clone());
            scheduler::start(app.handle().clone(), state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_host,
            commands::delete_host,
            commands::get_history,
            commands::list_hosts,
            commands::update_host_interval
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
