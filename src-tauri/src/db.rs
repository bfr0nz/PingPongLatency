use crate::models::{Host, HostSummary, PingResult, PingSample};
use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const CURRENT_SCHEMA_VERSION: i64 = 1;

pub fn initialize_database(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = Connection::open(path)?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        ",
    )?;
    migrate(&mut conn)?;
    Ok(())
}

pub fn open_connection(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|err| err.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|err| err.to_string())?;
    Ok(conn)
}

pub fn add_host(
    conn: &Connection,
    target: String,
    label: Option<String>,
    interval_seconds: i64,
) -> Result<Host, String> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO hosts (target, label, interval_seconds, enabled, created_at)
         VALUES (?1, ?2, ?3, 1, ?4)",
        params![target, label, interval_seconds, now],
    )
    .map_err(|err| err.to_string())?;

    get_host_by_id(conn, conn.last_insert_rowid())
}

pub fn delete_host(conn: &Connection, host_id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM hosts WHERE id = ?1", params![host_id])
        .map_err(|err| err.to_string())?;
    Ok(())
}

pub fn update_host_interval(
    conn: &Connection,
    host_id: i64,
    interval_seconds: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE hosts SET interval_seconds = ?1 WHERE id = ?2",
        params![interval_seconds, host_id],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

pub fn get_enabled_hosts(conn: &Connection) -> Result<Vec<Host>, String> {
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

pub fn get_due_hosts(conn: &Connection) -> Result<Vec<Host>, String> {
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

pub fn get_history(
    conn: &Connection,
    host_id: i64,
    window_minutes: i64,
) -> Result<Vec<PingResult>, String> {
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

pub fn summarize_hosts(
    conn: &Connection,
    window_minutes: i64,
) -> Result<Vec<HostSummary>, String> {
    get_enabled_hosts(conn)?
        .into_iter()
        .map(|host| summarize_host(conn, host, window_minutes))
        .collect()
}

pub fn insert_ping_result(
    conn: &Connection,
    host: &Host,
    sample: &PingSample,
) -> Result<PingResult, String> {
    let checked_at = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO ping_results (host_id, target, checked_at, latency_ms, success, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            host.id,
            host.target,
            checked_at,
            sample.latency_ms,
            sample.success,
            sample.error
        ],
    )
    .map_err(|err| err.to_string())?;

    Ok(PingResult {
        id: conn.last_insert_rowid(),
        host_id: host.id,
        target: host.target.clone(),
        checked_at,
        latency_ms: sample.latency_ms,
        success: sample.success,
        error: sample.error.clone(),
    })
}

pub fn mark_host_checked(conn: &Connection, host_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE hosts SET last_checked_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), host_id],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

pub fn prune_history(conn: &Connection) -> Result<(), String> {
    let cutoff = (Utc::now() - Duration::hours(13)).to_rfc3339();
    conn.execute(
        "DELETE FROM ping_results WHERE checked_at < ?1",
        params![cutoff],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current_version > CURRENT_SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidQuery);
    }

    if current_version == 0 {
        let transaction = conn.transaction()?;
        transaction.execute_batch(
            "
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

            PRAGMA user_version = 1;
            ",
        )?;
        transaction.commit()?;
    }

    Ok(())
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

fn since_for_window(minutes: i64) -> String {
    let minutes = minutes.clamp(1, 720);
    (Utc::now() - Duration::minutes(minutes)).to_rfc3339()
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
