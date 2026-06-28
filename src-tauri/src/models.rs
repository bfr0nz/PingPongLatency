use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Host {
    pub id: i64,
    pub target: String,
    pub label: Option<String>,
    pub interval_seconds: i64,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PingResult {
    pub id: i64,
    pub host_id: i64,
    pub target: String,
    pub checked_at: String,
    pub latency_ms: Option<f64>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HostSummary {
    pub host: Host,
    pub latest: Option<PingResult>,
    pub avg_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub packet_loss_percent: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct PingEvent {
    pub result: PingResult,
}

#[derive(Debug, Clone)]
pub struct PingSample {
    pub latency_ms: Option<f64>,
    pub success: bool,
    pub error: Option<String>,
}
