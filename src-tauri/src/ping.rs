use crate::models::PingSample;
use regex::Regex;
use std::{process::Stdio, sync::OnceLock};
use tokio::process::Command;

pub async fn ping_once(target: &str) -> PingSample {
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
    static LATENCY_RE: OnceLock<Regex> = OnceLock::new();
    let re = LATENCY_RE.get_or_init(|| {
        Regex::new(r"time[=<]\s*(\d+(?:\.\d+)?)\s*ms").expect("valid latency regex")
    });

    re.captures(output)
        .and_then(|caps| caps.get(1))
        .and_then(|value| value.as_str().parse::<f64>().ok())
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
