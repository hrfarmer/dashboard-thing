import { describe, expect, it } from "vitest";
import {
  normalizeGitHubJobs,
  normalizeGitHubRunners,
  normalizeTailscaleDevices,
} from "./normalizers";

describe("normalizers", () => {
  it("maps GitHub runner online and busy state", () => {
    const [runner] = normalizeGitHubRunners("org/example", [
      {
        id: 23,
        name: "linux-runner",
        os: "linux",
        status: "online",
        busy: true,
        labels: [{ name: "self-hosted" }, { name: "Linux" }],
      },
    ]);

    expect(runner).toMatchObject({
      id: "github:org/example:23",
      status: "online",
      busy: true,
      labels: ["self-hosted", "Linux"],
    });
  });

  it("splits GitHub current and queued jobs", () => {
    const result = normalizeGitHubJobs("org/example", "org/example", [
      {
        id: 1,
        name: "build",
        status: "in_progress",
        runner_id: 23,
        runner_name: "linux-runner",
        workflow_name: "CI",
      },
      {
        id: 2,
        name: "test",
        status: "queued",
        workflow_name: "CI",
      },
      {
        id: 3,
        name: "done",
        status: "completed",
      },
    ]);

    expect(result.currentJobs).toHaveLength(1);
    expect(result.queuedJobs).toHaveLength(1);
    expect(result.currentJobs[0].runnerId).toBe("github:org/example:23");
  });

  it("normalizes Tailscale status JSON", () => {
    const devices = normalizeTailscaleDevices(
      {
        Self: {
          ID: "self",
          HostName: "dashboard",
          TailscaleIPs: ["100.64.0.1"],
          Online: true,
        },
        Peer: {
          abc: {
            ID: "abc",
            HostName: "lab-runner-01",
            DNSName: "lab-runner-01.tailnet.ts.net.",
            OS: "linux",
            Online: false,
            LastSeen: "2026-05-05T22:00:00Z",
            User: "lab@example.com",
          },
        },
      },
      [{ name: "lab-runner-01", sshUser: "lab", restartable: true }],
    );

    expect(devices).toHaveLength(2);
    expect(devices.find((device) => device.name === "lab-runner-01")).toMatchObject({
      restartable: true,
      sshUser: "lab",
      online: false,
    });
  });
});
