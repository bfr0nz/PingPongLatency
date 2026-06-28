use crate::{
    db,
    models::{Host, PingEvent, PingSample},
    ping, AppState,
};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::{sync::Semaphore, task::JoinSet, time};

const MAX_CONCURRENT_PINGS: usize = 8;

pub fn start(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = time::interval(time::Duration::from_secs(1));
        loop {
            ticker.tick().await;
            if let Err(err) = run_due_pings(&app, &state).await {
                if err != "Previous scheduler tick still running" {
                    log::error!("Ping scheduler failed: {err}");
                }
            }
        }
    });
}

async fn run_due_pings(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let _lock = state
        .scheduler
        .try_lock()
        .map_err(|_| "Previous scheduler tick still running".to_string())?;

    let due_hosts = {
        let conn = db::open_connection(&state.db_path)?;
        db::get_due_hosts(&conn)?
    };

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PINGS));
    let mut jobs = JoinSet::new();

    for host in due_hosts {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| err.to_string())?;
        jobs.spawn(async move {
            let _permit = permit;
            let sample = ping::ping_once(&host.target).await;
            (host, sample)
        });
    }

    while let Some(result) = jobs.join_next().await {
        let (host, sample) = result.map_err(|err| err.to_string())?;
        persist_sample(app, state, &host, &sample)?;
    }

    let conn = db::open_connection(&state.db_path)?;
    db::prune_history(&conn)?;
    Ok(())
}

fn persist_sample(
    app: &AppHandle,
    state: &AppState,
    host: &Host,
    sample: &PingSample,
) -> Result<(), String> {
    let conn = db::open_connection(&state.db_path)?;
    let result = db::insert_ping_result(&conn, host, sample)?;
    db::mark_host_checked(&conn, host.id)?;
    log_sample(host, sample);

    app.emit("ping_result", PingEvent { result })
        .map_err(|err| err.to_string())?;

    Ok(())
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
