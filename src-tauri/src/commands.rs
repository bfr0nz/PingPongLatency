use crate::{
    db,
    models::{Host, HostSummary, PingResult},
    AppState,
};

#[tauri::command(rename_all = "camelCase")]
pub fn add_host(
    state: tauri::State<AppState>,
    target: String,
    label: Option<String>,
    interval_seconds: i64,
) -> Result<Host, String> {
    let target = normalize_target(&target)?;
    let interval_seconds = normalize_interval(interval_seconds)?;
    let label = label.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let conn = db::open_connection(&state.db_path)?;
    db::add_host(&conn, target, label, interval_seconds)
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_host(state: tauri::State<AppState>, host_id: i64) -> Result<(), String> {
    let conn = db::open_connection(&state.db_path)?;
    db::delete_host(&conn, host_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_host_interval(
    state: tauri::State<AppState>,
    host_id: i64,
    interval_seconds: i64,
) -> Result<(), String> {
    let interval_seconds = normalize_interval(interval_seconds)?;
    let conn = db::open_connection(&state.db_path)?;
    db::update_host_interval(&conn, host_id, interval_seconds)
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_hosts(
    state: tauri::State<AppState>,
    window_minutes: i64,
) -> Result<Vec<HostSummary>, String> {
    let conn = db::open_connection(&state.db_path)?;
    db::summarize_hosts(&conn, window_minutes)
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_history(
    state: tauri::State<AppState>,
    host_id: i64,
    window_minutes: i64,
) -> Result<Vec<PingResult>, String> {
    let conn = db::open_connection(&state.db_path)?;
    db::get_history(&conn, host_id, window_minutes)
}

fn normalize_target(target: &str) -> Result<String, String> {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return Err("Target is required".to_string());
    }
    if target.len() > 253 {
        return Err("Target is too long".to_string());
    }
    if target
        .chars()
        .all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '-' | ':'))
    {
        Ok(target)
    } else {
        Err("Use a hostname, IPv4 address, or IPv6 address".to_string())
    }
}

fn normalize_interval(interval_seconds: i64) -> Result<i64, String> {
    match interval_seconds {
        1 | 2 | 5 | 10 => Ok(interval_seconds),
        _ => Err("Interval must be 1, 2, 5, or 10 seconds".to_string()),
    }
}
