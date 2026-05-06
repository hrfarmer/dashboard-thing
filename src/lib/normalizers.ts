import type { Job, Runner, TailscaleDevice } from "./types";

type GitHubRunner = {
  id: number | string;
  name: string;
  os?: string;
  status?: string;
  busy?: boolean;
  labels?: Array<{ name?: string } | string>;
};

type GitHubJob = {
  id: number | string;
  name?: string;
  status?: string;
  html_url?: string;
  labels?: string[];
  runner_id?: number | string | null;
  runner_name?: string | null;
  workflow_name?: string;
  head_branch?: string;
  head_sha?: string;
  started_at?: string | null;
  created_at?: string | null;
};

type ForgejoJob = Record<string, unknown>;

export function normalizeGitHubRunners(
  source: string,
  runners: GitHubRunner[],
): Runner[] {
  return runners.map((runner) => ({
    id: `github:${source}:${runner.id}`,
    provider: "github",
    source,
    name: runner.name,
    os: runner.os,
    status: runner.status === "online" ? "online" : runner.status === "offline" ? "offline" : "unknown",
    busy: Boolean(runner.busy),
    labels: (runner.labels ?? []).map((label) =>
      typeof label === "string" ? label : label.name ?? "",
    ).filter(Boolean),
  }));
}

export function normalizeGitHubJobs(source: string, repository: string, jobs: GitHubJob[]): {
  currentJobs: Job[];
  queuedJobs: Job[];
} {
  const normalized = jobs
    .filter((job) => isActiveJobStatus(job.status))
    .map((job) => ({
      id: `github:${repository}:${job.id}`,
      provider: "github" as const,
      source,
      repository,
      workflow: job.workflow_name ?? "Workflow",
      name: job.name ?? "Job",
      status: normalizeJobStatus(job.status),
      runnerId: job.runner_id ? `github:${source}:${job.runner_id}` : undefined,
      runnerName: job.runner_name ?? undefined,
      branch: job.head_branch,
      sha: shortSha(job.head_sha),
      url: job.html_url,
      startedAt: job.started_at ?? undefined,
      queuedAt: job.created_at ?? undefined,
    }));

  return {
    currentJobs: normalized.filter((job) => job.status === "in_progress" || job.status === "running"),
    queuedJobs: normalized.filter((job) => job.status !== "in_progress" && job.status !== "running"),
  };
}

export function normalizeForgejoJobs(source: string, repository: string, jobs: ForgejoJob[]): {
  currentJobs: Job[];
  queuedJobs: Job[];
} {
  const normalized = jobs
    .map((job) => {
      const id = String(pick(job, ["id", "job_id", "run_id"]) ?? cryptoRandomId());
      const status = normalizeJobStatus(String(pick(job, ["status", "state"]) ?? "unknown"));
      return {
        id: `forgejo:${repository}:${id}`,
        provider: "forgejo" as const,
        source,
        repository,
        workflow: String(pick(job, ["workflow_name", "workflow", "name"]) ?? "Workflow"),
        name: String(pick(job, ["job_name", "display_title", "name"]) ?? "Job"),
        status,
        runnerId: optionalString(pick(job, ["runner_id"])),
        runnerName: optionalString(pick(job, ["runner_name", "runner"])),
        branch: optionalString(pick(job, ["head_branch", "ref_name", "branch"])),
        sha: shortSha(optionalString(pick(job, ["head_sha", "commit_sha", "sha"]))),
        url: optionalString(pick(job, ["html_url", "url"])),
        startedAt: optionalString(pick(job, ["started_at", "start_time"])),
        queuedAt: optionalString(pick(job, ["created_at", "queued_at"])),
      };
    })
    .filter((job) => isActiveJobStatus(job.status));

  return {
    currentJobs: normalized.filter((job) => job.status === "in_progress" || job.status === "running"),
    queuedJobs: normalized.filter((job) => job.status !== "in_progress" && job.status !== "running"),
  };
}

export function normalizeTailscaleDevices(
  status: Record<string, unknown>,
  configuredDevices: Array<{ name: string; sshUser?: string; restartable?: boolean }> = [],
): TailscaleDevice[] {
  const peers = Object.values((status.Peer ?? {}) as Record<string, Record<string, unknown>>);
  const self = status.Self && typeof status.Self === "object" ? [status.Self as Record<string, unknown>] : [];
  const configuredByName = new Map(configuredDevices.map((device) => [device.name, device]));

  return [...self, ...peers].map((peer) => {
    const hostName = String(peer.HostName ?? peer.DNSName ?? "unknown");
    const dnsName = optionalString(peer.DNSName);
    const displayName = trimTailnetName(dnsName) ?? hostName;
    const configured = configuredByName.get(displayName) ?? configuredByName.get(hostName);
    const tailscaleIps = Array.isArray(peer.TailscaleIPs) ? peer.TailscaleIPs : [];
    const tags = Array.isArray(peer.Tags) ? peer.Tags.map(String) : [];

    return {
      id: String(peer.ID ?? peer.PublicKey ?? displayName),
      name: displayName,
      hostName,
      dnsName,
      tailnetIp: tailscaleIps.length > 0 ? String(tailscaleIps[0]) : undefined,
      os: optionalString(peer.OS),
      online: Boolean(peer.Online),
      lastSeen: optionalString(peer.LastSeen),
      user: optionalString(peer.User),
      tags,
      sshUser: configured?.sshUser,
      restartable: Boolean(configured?.restartable),
    };
  }).sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name));
}

export function isActiveJobStatus(status: unknown): boolean {
  return ["queued", "waiting", "pending", "in_progress", "running", "requested"].includes(
    normalizeJobStatus(String(status ?? "unknown")),
  );
}

export function normalizeJobStatus(status: string | undefined): Job["status"] {
  switch ((status ?? "").toLowerCase()) {
    case "queued":
    case "requested":
      return "queued";
    case "waiting":
      return "waiting";
    case "pending":
      return "pending";
    case "in_progress":
    case "in-progress":
      return "in_progress";
    case "running":
      return "running";
    default:
      return "unknown";
  }
}

function pick(record: Record<string, unknown>, keys: string[]): unknown {
  for (const key of keys) {
    if (record[key] !== undefined && record[key] !== null) return record[key];
  }
  return undefined;
}

function optionalString(value: unknown): string | undefined {
  return value === undefined || value === null || value === "" ? undefined : String(value);
}

function shortSha(value: string | undefined): string | undefined {
  return value ? value.slice(0, 7) : undefined;
}

function trimTailnetName(value: string | undefined): string | undefined {
  if (!value) return undefined;
  return value.replace(/\.$/, "").split(".")[0];
}

function cryptoRandomId(): string {
  return Math.random().toString(36).slice(2);
}
