import React from "react";
import ReactDOM from "react-dom/client";
import { Activity, ExternalLink, Power, RefreshCw, Terminal } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import "./styles.css";
import type { ActionResult, DashboardData, Job, Runner, SourceStatus, TailscaleDevice } from "./lib/types";

const emptyData: DashboardData = {
  updatedAt: new Date().toISOString(),
  refreshIntervalSeconds: 30,
  sources: [],
  runners: [],
  currentJobs: [],
  queuedJobs: [],
  devices: [],
};

function App() {
  const [data, setData] = React.useState<DashboardData>(emptyData);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [actionMessage, setActionMessage] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setError(null);
      const next = await invoke<DashboardData>("get_dashboard_data");
      setData(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  React.useEffect(() => {
    const interval = window.setInterval(() => {
      void refresh();
    }, Math.max(data.refreshIntervalSeconds, 10) * 1000);
    return () => window.clearInterval(interval);
  }, [data.refreshIntervalSeconds, refresh]);

  async function runDeviceAction(action: "open_ssh" | "restart_device", device: TailscaleDevice) {
    if (action === "restart_device") {
      const confirmed = window.confirm(`Restart ${device.name}? This will run the configured reboot command over Tailscale SSH.`);
      if (!confirmed) return;
    }

    try {
      setActionMessage(null);
      const result = await invoke<ActionResult>(action, { deviceName: device.name });
      setActionMessage(result.message);
      await refresh();
    } catch (err) {
      setActionMessage(err instanceof Error ? err.message : String(err));
    }
  }

  return (
    <main className="shell">
      <header className="topbar">
        <div>
          <h1>TXST Lab Dashboard</h1>
          <p>{loading ? "Loading" : `Updated ${formatDateTime(data.updatedAt)}`}</p>
        </div>
        <button className="button" onClick={() => void refresh()} type="button">
          <RefreshCw size={16} aria-hidden />
          Refresh
        </button>
      </header>

      {error ? <div className="notice notice-error">{error}</div> : null}
      {actionMessage ? <div className="notice">{actionMessage}</div> : null}

      <section className="source-strip" aria-label="Source status">
        {data.sources.length === 0 ? (
          <SourcePill source={{ provider: "github", name: "No configured sources", health: "warning", message: "Add dashboard.config.json" }} />
        ) : (
          data.sources.map((source) => <SourcePill key={`${source.provider}-${source.name}`} source={source} />)
        )}
      </section>

      <div className="dashboard-grid">
        <div className="column">
          <Panel title="Runners">
            <RunnerTable runners={data.runners} jobs={data.currentJobs} />
          </Panel>
          <Panel title="Current Jobs">
            <JobTable jobs={data.currentJobs} emptyText="No jobs are currently running." />
          </Panel>
          <Panel title="Queued Jobs">
            <JobTable jobs={data.queuedJobs} emptyText="No queued jobs." />
          </Panel>
        </div>

        <div className="column column-narrow">
          <Panel title="Tailscale Devices">
            <DeviceList devices={data.devices} onAction={runDeviceAction} />
          </Panel>
        </div>
      </div>
    </main>
  );
}

function SourcePill({ source }: { source: SourceStatus }) {
  return (
    <div className="source-item" data-health={source.health}>
      <span>{source.name}</span>
      <strong>{source.health}</strong>
      {source.message ? <em>{source.message}</em> : null}
    </div>
  );
}

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="panel">
      <div className="panel-header">
        <h2>{title}</h2>
      </div>
      {children}
    </section>
  );
}

function RunnerTable({ runners, jobs }: { runners: Runner[]; jobs: Job[] }) {
  if (runners.length === 0) return <EmptyState text="No runners found for the configured sources." />;
  const jobByRunner = new Map(jobs.flatMap((job) => job.runnerId ? [[job.runnerId, job]] : []));

  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Provider</th>
            <th>Name</th>
            <th>Status</th>
            <th>OS</th>
            <th>Labels</th>
            <th>Current job</th>
          </tr>
        </thead>
        <tbody>
          {runners.map((runner) => {
            const job = jobByRunner.get(runner.id);
            return (
              <tr key={runner.id}>
                <td>{providerLabel(runner.provider)}</td>
                <td>{runner.name}</td>
                <td><StatusText tone={runner.status === "online" ? "good" : "bad"} text={runner.busy ? "busy" : runner.status} /></td>
                <td>{runner.os ?? "Unknown"}</td>
                <td className="muted">{runner.labels.join(", ") || "None"}</td>
                <td>{job ? job.name : "Idle"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function JobTable({ jobs, emptyText }: { jobs: Job[]; emptyText: string }) {
  if (jobs.length === 0) return <EmptyState text={emptyText} />;
  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Provider</th>
            <th>Repository</th>
            <th>Workflow</th>
            <th>Job</th>
            <th>Status</th>
            <th>Runner</th>
            <th>Branch</th>
            <th>Started</th>
            <th>Link</th>
          </tr>
        </thead>
        <tbody>
          {jobs.map((job) => (
            <tr key={job.id}>
              <td>{providerLabel(job.provider)}</td>
              <td>{job.repository}</td>
              <td>{job.workflow}</td>
              <td>{job.name}</td>
              <td><StatusText tone={job.status === "queued" ? "warn" : "good"} text={job.status.replace("_", " ")} /></td>
              <td>{job.runnerName ?? "Unassigned"}</td>
              <td>{[job.branch, job.sha].filter(Boolean).join(" @ ") || "Unknown"}</td>
              <td>{formatDateTime(job.startedAt ?? job.queuedAt)}</td>
              <td>
                {job.url ? (
                  <a className="icon-link" href={job.url} target="_blank" rel="noreferrer" aria-label={`Open ${job.name}`}>
                    <ExternalLink size={15} aria-hidden />
                  </a>
                ) : "None"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function DeviceList({
  devices,
  onAction,
}: {
  devices: TailscaleDevice[];
  onAction: (action: "open_ssh" | "restart_device", device: TailscaleDevice) => void;
}) {
  if (devices.length === 0) return <EmptyState text="No Tailscale devices found. Confirm the CLI is logged in." />;
  return (
    <div className="device-list">
      {devices.map((device) => (
        <article className="device-row" key={device.id}>
          <div className="device-main">
            <div className="device-title">
              <strong>{device.name}</strong>
              <StatusText tone={device.online ? "good" : "bad"} text={device.online ? "online" : "offline"} />
            </div>
            <div className="device-meta">
              <span>{device.tailnetIp ?? "No Tailnet IP"}</span>
              <span>{device.os ?? "Unknown OS"}</span>
              <span>{device.user ?? "No user"}</span>
            </div>
            <div className="device-meta">
              <span>{device.dnsName ?? device.hostName}</span>
              <span>Last seen {formatDateTime(device.lastSeen)}</span>
            </div>
          </div>
          <div className="device-actions">
            <button className="icon-button" onClick={() => onAction("open_ssh", device)} type="button" title="Open SSH">
              <Terminal size={16} aria-hidden />
            </button>
            {device.restartable ? (
              <button className="icon-button danger" onClick={() => onAction("restart_device", device)} type="button" title="Restart">
                <Power size={16} aria-hidden />
              </button>
            ) : null}
          </div>
        </article>
      ))}
    </div>
  );
}

function StatusText({ tone, text }: { tone: "good" | "warn" | "bad"; text: string }) {
  return <span className={`status-text ${tone}`}>{text}</span>;
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="empty">
      <Activity size={16} aria-hidden />
      {text}
    </div>
  );
}

function providerLabel(provider: string): string {
  return provider === "github" ? "GitHub" : provider === "forgejo" ? "Forgejo" : provider;
}

function formatDateTime(value?: string): string {
  if (!value) return "Unknown";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Unknown";
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
