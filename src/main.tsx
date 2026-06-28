import React, { useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Activity, Clock, Globe2, Plus, Trash2, WifiOff } from "lucide-react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis
} from "recharts";
import { format } from "date-fns";
import "./styles.css";

type Host = {
  id: number;
  target: string;
  label: string | null;
  interval_seconds: number;
  enabled: boolean;
  created_at: string;
};

type PingResult = {
  id: number;
  host_id: number;
  target: string;
  checked_at: string;
  latency_ms: number | null;
  success: boolean;
  error: string | null;
};

type HostSummary = {
  host: Host;
  latest: PingResult | null;
  avg_latency_ms: number | null;
  max_latency_ms: number | null;
  packet_loss_percent: number;
};

type PingEvent = {
  result: PingResult;
};

type WindowOption = {
  label: string;
  minutes: number;
};

const WINDOW_OPTIONS: WindowOption[] = [
  { label: "1m", minutes: 1 },
  { label: "5m", minutes: 5 },
  { label: "15m", minutes: 15 },
  { label: "1h", minutes: 60 },
  { label: "6h", minutes: 360 },
  { label: "12h", minutes: 720 }
];

const INTERVALS = [1, 2, 5, 10];

function App() {
  const [hosts, setHosts] = useState<HostSummary[]>([]);
  const [selectedHostId, setSelectedHostId] = useState<number | null>(null);
  const [history, setHistory] = useState<PingResult[]>([]);
  const [target, setTarget] = useState("");
  const [label, setLabel] = useState("");
  const [intervalSeconds, setIntervalSeconds] = useState(2);
  const [windowMinutes, setWindowMinutes] = useState(15);
  const [isAdding, setIsAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pingVersion, setPingVersion] = useState(0);

  async function refreshHosts() {
    const nextHosts = await invoke<HostSummary[]>("list_hosts", { windowMinutes });
    setHosts(nextHosts);
    setSelectedHostId((current) => current ?? nextHosts[0]?.host.id ?? null);
  }

  async function refreshHistory(hostId: number | null = selectedHostId) {
    if (!hostId) {
      setHistory([]);
      return;
    }
    const rows = await invoke<PingResult[]>("get_history", { hostId, windowMinutes });
    setHistory(rows);
  }

  useEffect(() => {
    refreshHosts().catch((err) => setError(String(err)));
  }, [windowMinutes]);

  useEffect(() => {
    refreshHistory().catch((err) => setError(String(err)));
  }, [selectedHostId, windowMinutes]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    listen<PingEvent>("ping_result", () => {
      setPingVersion((version) => version + 1);
    })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
        } else {
          unlisten = nextUnlisten;
        }
      })
      .catch((err) => setError(String(err)));

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (pingVersion === 0) return;
    refreshHosts().catch((err) => setError(String(err)));
    refreshHistory().catch((err) => setError(String(err)));
  }, [pingVersion, selectedHostId, windowMinutes]);

  async function addHost(event: React.FormEvent) {
    event.preventDefault();
    if (!target.trim()) return;
    setIsAdding(true);
    setError(null);
    try {
      const host = await invoke<Host>("add_host", {
        target: target.trim(),
        label: label.trim() || null,
        intervalSeconds
      });
      setTarget("");
      setLabel("");
      setSelectedHostId(host.id);
      await refreshHosts();
    } catch (err) {
      setError(String(err));
    } finally {
      setIsAdding(false);
    }
  }

  async function removeHost(hostId: number) {
    await invoke("delete_host", { hostId });
    setSelectedHostId((current) => (current === hostId ? null : current));
    await refreshHosts();
  }

  async function updateInterval(hostId: number, seconds: number) {
    await invoke("update_host_interval", { hostId, intervalSeconds: seconds });
    await refreshHosts();
  }

  const chartData = useMemo(
    () =>
      history.map((row) => ({
        time: new Date(row.checked_at).getTime(),
        label: format(new Date(row.checked_at), windowMinutes <= 60 ? "HH:mm:ss" : "HH:mm"),
        latency: row.success ? row.latency_ms : null,
        status: row.success ? "ok" : row.error ?? "failed"
      })),
    [history, windowMinutes]
  );

  const selectedSummary = hosts.find((item) => item.host.id === selectedHostId) ?? null;

  return (
    <main className="app-shell">
      <section className="toolbar">
        <div>
          <h1>Ping Pong Latency</h1>
          <p>Desktop latency monitor for IP addresses and hostnames.</p>
        </div>
        <div className="window-picker" aria-label="Time window">
          {WINDOW_OPTIONS.map((option) => (
            <button
              key={option.minutes}
              className={option.minutes === windowMinutes ? "active" : ""}
              onClick={() => setWindowMinutes(option.minutes)}
            >
              {option.label}
            </button>
          ))}
        </div>
      </section>

      <section className="content-grid">
        <aside className="sidebar">
          <form className="add-host" onSubmit={addHost}>
            <label>
              Target
              <input
                value={target}
                onChange={(event) => setTarget(event.target.value)}
                placeholder="1.1.1.1 or example.com"
              />
            </label>
            <label>
              Label
              <input
                value={label}
                onChange={(event) => setLabel(event.target.value)}
                placeholder="Optional"
              />
            </label>
            <label>
              Interval
              <select
                value={intervalSeconds}
                onChange={(event) => setIntervalSeconds(Number(event.target.value))}
              >
                {INTERVALS.map((seconds) => (
                  <option key={seconds} value={seconds}>
                    {seconds}s
                  </option>
                ))}
              </select>
            </label>
            <button className="primary" disabled={isAdding}>
              <Plus size={16} />
              Add
            </button>
          </form>

          <div className="host-list">
            {hosts.map(({ host, latest, packet_loss_percent }) => (
              <button
                key={host.id}
                className={`host-row ${selectedHostId === host.id ? "selected" : ""}`}
                onClick={() => setSelectedHostId(host.id)}
              >
                <span className="host-title">
                  <Globe2 size={16} />
                  <span>{host.label || host.target}</span>
                </span>
                <span className="host-meta">
                  {latest?.success && latest.latency_ms != null
                    ? `${latest.latency_ms.toFixed(1)} ms`
                    : "offline"}
                  <span>{packet_loss_percent.toFixed(0)}% loss</span>
                </span>
              </button>
            ))}
          </div>
        </aside>

        <section className="monitor">
          {selectedSummary ? (
            <>
              <div className="host-header">
                <div>
                  <h2>{selectedSummary.host.label || selectedSummary.host.target}</h2>
                  <p>{selectedSummary.host.target}</p>
                </div>
                <div className="host-actions">
                  <select
                    value={selectedSummary.host.interval_seconds}
                    onChange={(event) =>
                      updateInterval(selectedSummary.host.id, Number(event.target.value))
                    }
                    aria-label="Ping interval"
                  >
                    {INTERVALS.map((seconds) => (
                      <option key={seconds} value={seconds}>
                        {seconds}s
                      </option>
                    ))}
                  </select>
                  <button
                    className="icon-button danger"
                    onClick={() => removeHost(selectedSummary.host.id)}
                    aria-label="Delete host"
                    title="Delete host"
                  >
                    <Trash2 size={18} />
                  </button>
                </div>
              </div>

              <div className="stats-grid">
                <Stat icon={<Activity size={18} />} label="Current" value={formatMs(selectedSummary.latest?.latency_ms)} />
                <Stat icon={<Clock size={18} />} label="Average" value={formatMs(selectedSummary.avg_latency_ms)} />
                <Stat icon={<Activity size={18} />} label="Max" value={formatMs(selectedSummary.max_latency_ms)} />
                <Stat icon={<WifiOff size={18} />} label="Packet loss" value={`${selectedSummary.packet_loss_percent.toFixed(1)}%`} />
              </div>

              <div className="chart-panel">
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={chartData} margin={{ top: 12, right: 24, bottom: 8, left: 0 }}>
                    <defs>
                      <linearGradient id="latencyFill" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="5%" stopColor="#0f8b8d" stopOpacity={0.35} />
                        <stop offset="95%" stopColor="#0f8b8d" stopOpacity={0.02} />
                      </linearGradient>
                    </defs>
                    <CartesianGrid stroke="#e2e8f0" vertical={false} />
                    <XAxis dataKey="label" minTickGap={32} stroke="#64748b" />
                    <YAxis stroke="#64748b" unit=" ms" width={72} />
                    <Tooltip
                      formatter={(value) => [`${Number(value).toFixed(1)} ms`, "Latency"]}
                      labelFormatter={(_, payload) => payload?.[0]?.payload?.label ?? ""}
                    />
                    <Area
                      type="monotone"
                      dataKey="latency"
                      stroke="#0f8b8d"
                      strokeWidth={2}
                      fill="url(#latencyFill)"
                      connectNulls={false}
                      isAnimationActive={false}
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
            </>
          ) : (
            <div className="empty-state">Add a target to start collecting latency samples.</div>
          )}
        </section>
      </section>

      {error ? <div className="error-bar">{error}</div> : null}
    </main>
  );
}

function Stat({ icon, label, value }: { icon: React.ReactNode; label: string; value: string }) {
  return (
    <div className="stat">
      <span>{icon}</span>
      <div>
        <p>{label}</p>
        <strong>{value}</strong>
      </div>
    </div>
  );
}

function formatMs(value: number | null | undefined) {
  return value == null ? "n/a" : `${value.toFixed(1)} ms`;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
