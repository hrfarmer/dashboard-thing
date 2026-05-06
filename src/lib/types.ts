export type Provider = "github" | "forgejo";

export type Health = "ok" | "warning" | "error";

export interface SourceStatus {
  provider: Provider | "tailscale";
  name: string;
  health: Health;
  message?: string;
}

export interface Runner {
  id: string;
  provider: Provider;
  source: string;
  name: string;
  os?: string;
  status: "online" | "offline" | "unknown";
  busy: boolean;
  labels: string[];
  currentJobId?: string;
}

export interface Job {
  id: string;
  provider: Provider;
  source: string;
  repository: string;
  workflow: string;
  name: string;
  status: "queued" | "waiting" | "pending" | "in_progress" | "running" | "unknown";
  runnerId?: string;
  runnerName?: string;
  branch?: string;
  sha?: string;
  url?: string;
  startedAt?: string;
  queuedAt?: string;
}

export interface TailscaleDevice {
  id: string;
  name: string;
  hostName: string;
  dnsName?: string;
  tailnetIp?: string;
  os?: string;
  online: boolean;
  lastSeen?: string;
  user?: string;
  tags: string[];
  sshUser?: string;
  restartable: boolean;
}

export interface DashboardData {
  updatedAt: string;
  refreshIntervalSeconds: number;
  sources: SourceStatus[];
  runners: Runner[];
  currentJobs: Job[];
  queuedJobs: Job[];
  devices: TailscaleDevice[];
}

export interface ActionResult {
  ok: boolean;
  message: string;
}
