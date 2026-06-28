use chrono::{Duration, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::Arc,
};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use tokio::{process::Command, time};

#[derive(Clone)]
struct AppState {
    db_path: Arc<PathBuf>,
    scheduler: Arc<Mutex<()>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Host {
    id: i64,
    target: String,
    label: Option<String>,
    interval_seconds: i64,
    enabled: bool,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PingResult {
    id: i64,
    host_id: i64,
    target: String,
    checked_at: String,
    latency_ms: Option<f64>,
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct HostSummary {
    host: Host,
    latest: Option<PingResult>,
    avg_latency_ms: Option<f64>,
    max_latency_ms: Option<f64>,
    packet_loss_percent: f64,
}

#[derive(Debug)]
struct PingSample {
    latency_ms: Option<f64>,
    success: bool,
    error: Option<String>,
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

            initialize_database(&db_path)?;

            let state = AppState {
                db_path: Arc::new(db_path),
                scheduler: Arc::new(Mutex::new(())),
            };

            app.manage(state.clone());
            start_scheduler(app.handle().clone(), state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            add_host,
            delete_host,
            get_history,
            list_hosts,
            update_host_interval
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command(rename_all = "camelCase")]
fn add_host(
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
    let now = Utc::now().to_rfc3339();
    let conn = open_connection(&state.db_path)?;

    conn.execute(
        "INSERT INTO hosts (target, label, interval_seconds, enabled, created_at)
         VALUES (?1, ?2, ?3, 1, ?4)",
        params![target, label, interval_seconds, now],
    )
    .map_err(|err| err.to_string())?;

    get_host_by_id(&conn, conn.last_insert_rowid())
}

#[tauri::command(rename_all = "camelCase")]
fn delete_host(state: tauri::State<AppState>, host_id: i64) -> Result<(), String> {
    let conn = open_connection(&state.db_path)?;
    conn.execute("DELETE FROM hosts WHERE id = ?1", params![host_id])
        .map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
fn update_host_interval(
    state: tauri::State<AppState>,
    host_id: i64,
    interval_seconds: i64,
) -> Result<(), String> {
    let interval_seconds = normalize_interval(interval_seconds)?;
    let conn = open_connection(&state.db_path)?;
    conn.execute(
        "UPDATE hosts SET interval_seconds = ?1 WHERE id = ?2",
        params![interval_seconds, host_id],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
fn list_hosts(state: tauri::State<AppState>, window_minutes: i64) -> Result<Vec<HostSummary>, String> {
    let conn = open_connection(&state.db_path)?;
    let hosts = get_enabled_hosts(&conn)?;
    hosts
        .into_iter()
        .map(|host| summarize_host(&conn, host, window_minutes))
        .collect()
}

#[tauri::command(rename_all = "camelCase")]
fn get_history(
    state: tauri::State<AppState>,
    host_id: i64,
    window_minutes: i64,
) -> Result<Vec<PingResult>, String> {
    let conn = open_connection(&state.db_path)?;
    let since = since_for_window(window_minutes);
    let mut stmt = conn
        .prepare(
            "SELECT id, host_id, target, checked_at, latency_ms, success, error
             FROM ping_results
             WHERE host_id = ?1 AND checked_at >= ?2
             ORDER BY checked_at ASC",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map(params![host_id, since], row_to_ping_result)
        .map_err(|err| err.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())
}

fn start_scheduler(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = time::interval(time::Duration::from_secs(1));
        loop {
            ticker.tick().await;
            if let Err(err) = run_due_pings(&app, &state).await {
                log::error!("Ping scheduler failed: {err}");
            }
        }
    });
}

async fn run_due_pings(_app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _lock = state
        .scheduler
        .try_lock()
        .map_err(|_| "Previous scheduler tick still running".to_string())?;

    let due_hosts = {
        let conn = open_connection(&state.db_path)?;
        get_due_hosts(&conn)?
    };

    for host in due_hosts {
        let sample = ping_once(&host.target).await;
        let conn = open_connection(&state.db_path)?;
        insert_ping_result(&conn, &host, &sample)?;
        mark_host_checked(&conn, host.id)?;
        log_sample(&host, &sample);
    }

    let conn = open_connection(&state.db_path)?;
    prune_history(&conn)?;
    Ok(())
}

async fn ping_once(target: &str) -> PingSample {
    #[cfg(target_os = "windows")]
    let args = ["-n", "1", "-w", "1500", target];

    #[cfg(not(target_os = "windows"))]
    let args = ["-c", "1", "-W", "2", target];

    let output = Command::new("ping")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            match parse_latency_ms(&stdout) {
                Some(latency_ms) => PingSample {
                    latency_ms: Some(latency_ms),
                    success: true,
                    error: None,
                },
                None => PingSample {
                    latency_ms: None,
                    success: false,
                    error: Some("Ping succeeded but latency could not be parsed".to_string()),
                },
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let message = first_non_empty(&stderr).or_else(|| first_non_empty(&stdout));
            PingSample {
                latency_ms: None,
                success: false,
                error: Some(message.unwrap_or_else(|| "Ping failed".to_string())),
            }
        }
        Err(err) => PingSample {
            latency_ms: None,
            success: false,
            error: Some(err.to_string()),
        },
    }
}

fn parse_latency_ms(output: &str) -> Option<f64> {
    let re = Regex::new(r"time[=<]\s*(\d+(?:\.\d+)?)\s*ms").ok()?;
    re.captures(output)
        .and_then(|caps| caps.get(1))
        .and_then(|value| value.as_str().parse::<f64>().ok())
}

fn initialize_database(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS hosts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            target TEXT NOT NULL UNIQUE,
            label TEXT,
            interval_seconds INTEGER NOT NULL DEFAULT 2,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            last_checked_at TEXT
        );

        CREATE TABLE IF NOT EXISTS ping_results (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            host_id INTEGER NOT NULL,
            target TEXT NOT NULL,
            checked_at TEXT NOT NULL,
            latency_ms REAL,
            success INTEGER NOT NULL,
            error TEXT,
            FOREIGN KEY (host_id) REFERENCES hosts(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_ping_results_host_time
        ON ping_results(host_id, checked_at);
        ",
    )?;
    Ok(())
}

fn open_connection(path: &PathBuf) -> Result<Connection, String> {
    Connection::open(path).map_err(|err| err.to_string())
}

fn get_enabled_hosts(conn: &Connection) -> Result<Vec<Host>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, target, label, interval_seconds, enabled, created_at
             FROM hosts
             WHERE enabled = 1
             ORDER BY target ASC",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map([], row_to_host)
        .map_err(|err| err.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())
}

fn get_due_hosts(conn: &Connection) -> Result<Vec<Host>, String> {
    let now = Utc::now().to_rfc3339();
    let mut stmt = conn
        .prepare(
            "SELECT id, target, label, interval_seconds, enabled, created_at
             FROM hosts
             WHERE enabled = 1
             AND (
                last_checked_at IS NULL
                OR datetime(last_checked_at) <= datetime(?1, '-' || interval_seconds || ' seconds')
             )
             ORDER BY last_checked_at ASC NULLS FIRST",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map(params![now], row_to_host)
        .map_err(|err| err.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())
}

fn get_host_by_id(conn: &Connection, id: i64) -> Result<Host, String> {
    conn.query_row(
        "SELECT id, target, label, interval_seconds, enabled, created_at
         FROM hosts
         WHERE id = ?1",
        params![id],
        row_to_host,
    )
    .map_err(|err| err.to_string())
}

fn summarize_host(conn: &Connection, host: Host, window_minutes: i64) -> Result<HostSummary, String> {
    let since = since_for_window(window_minutes);
    let latest = conn
        .query_row(
            "SELECT id, host_id, target, checked_at, latency_ms, success, error
             FROM ping_results
             WHERE host_id = ?1
             ORDER BY checked_at DESC
             LIMIT 1",
            params![host.id],
            row_to_ping_result,
        )
        .optional()
        .map_err(|err| err.to_string())?;

    let (avg_latency_ms, max_latency_ms, total_count, failed_count): (
        Option<f64>,
        Option<f64>,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT AVG(latency_ms), MAX(latency_ms), COUNT(*), COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0)
             FROM ping_results
             WHERE host_id = ?1 AND checked_at >= ?2",
            params![host.id, since],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|err| err.to_string())?;

    let packet_loss_percent = if total_count == 0 {
        0.0
    } else {
        (failed_count as f64 / total_count as f64) * 100.0
    };

    Ok(HostSummary {
        host,
        latest,
        avg_latency_ms,
        max_latency_ms,
        packet_loss_percent,
    })
}

fn insert_ping_result(conn: &Connection, host: &Host, sample: &PingSample) -> Result<(), String> {
    conn.execute(
        "INSERT INTO ping_results (host_id, target, checked_at, latency_ms, success, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            host.id,
            host.target,
            Utc::now().to_rfc3339(),
            sample.latency_ms,
            sample.success,
            sample.error
        ],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn mark_host_checked(conn: &Connection, host_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE hosts SET last_checked_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), host_id],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn prune_history(conn: &Connection) -> Result<(), String> {
    let cutoff = (Utc::now() - Duration::hours(13)).to_rfc3339();
    conn.execute(
        "DELETE FROM ping_results WHERE checked_at < ?1",
        params![cutoff],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn since_for_window(minutes: i64) -> String {
    let minutes = minutes.clamp(1, 720);
    (Utc::now() - Duration::minutes(minutes)).to_rfc3339()
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

fn row_to_host(row: &rusqlite::Row<'_>) -> rusqlite::Result<Host> {
    Ok(Host {
        id: row.get(0)?,
        target: row.get(1)?,
        label: row.get(2)?,
        interval_seconds: row.get(3)?,
        enabled: row.get::<_, i64>(4)? == 1,
        created_at: row.get(5)?,
    })
}

fn row_to_ping_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<PingResult> {
    Ok(PingResult {
        id: row.get(0)?,
        host_id: row.get(1)?,
        target: row.get(2)?,
        checked_at: row.get(3)?,
        latency_ms: row.get(4)?,
        success: row.get::<_, i64>(5)? == 1,
        error: row.get(6)?,
    })
}

fn log_sample(host: &Host, sample: &PingSample) {
    match (sample.success, sample.latency_ms) {
        (true, Some(latency)) if latency >= 250.0 => {
            log::warn!("High latency for {}: {:.1} ms", host.target, latency);
        }
        (true, Some(latency)) => {
            log::info!("Ping {}: {:.1} ms", host.target, latency);
        }
        _ => {
            log::warn!(
                "Ping failed for {}: {}",
                host.target,
                sample.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
}

fn first_non_empty(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_latency() {
        let output = "Reply from 1.1.1.1: bytes=32 time=11ms TTL=58";
        assert_eq!(parse_latency_ms(output), Some(11.0));
    }

    #[test]
    fn parses_sub_millisecond_windows_latency() {
        let output = "Reply from 192.168.1.1: bytes=32 time<1ms TTL=64";
        assert_eq!(parse_latency_ms(output), Some(1.0));
    }

    #[test]
    fn parses_unix_latency() {
        let output = "64 bytes from 1.1.1.1: icmp_seq=0 ttl=57 time=8.432 ms";
        assert_eq!(parse_latency_ms(output), Some(8.432));
    }
}
