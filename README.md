# TXST Lab Dashboard

A local Tauri dashboard for lab machines, self-hosted CI runners, queued jobs, and Tailscale devices.

## Run

```sh
npm install
npm run tauri dev
```

Build for the current machine:

```sh
npm run build:app
```

Build on macOS:

```sh
npm run build:mac
```

Build on Linux:

```sh
npm run build:linux
```

The Linux build emits the configured Tauri Linux bundles, currently Debian package, AppImage, and RPM when the required system packaging tools are available. Tauri Linux builds need the usual WebKitGTK and appindicator development packages installed on the build machine.

On Debian or Ubuntu, install the common prerequisites with:

```sh
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf build-essential curl wget file
```

## Configure

Copy the example config and edit it for the lab:

```sh
cp dashboard.config.example.json dashboard.config.json
```

Secrets are read from environment variables, not from `dashboard.config.json`:

```sh
export GITHUB_TOKEN=...
export CODEBERG_TOKEN=...
```

Config lookup order:

1. `TXST_DASHBOARD_CONFIG`, if set to a file path.
2. `dashboard.config.json` in the current working directory.
3. `dashboard.config.json` next to the AppImage on Linux.
4. `dashboard.config.json` next to the app executable.
5. `$XDG_CONFIG_HOME/txst-lab-dashboard/dashboard.config.json`.
6. `$HOME/.config/txst-lab-dashboard/dashboard.config.json`.
7. `/etc/txst-lab-dashboard/dashboard.config.json`.
8. Bundled app resources.

When running from source, keep `dashboard.config.json` at the repository root. When running the Linux AppImage, either place `dashboard.config.json` beside the AppImage or launch it with:

```sh
TXST_DASHBOARD_CONFIG=/path/to/dashboard.config.json ./TXST\ Lab\ Dashboard_0.1.0_amd64.AppImage
```

## Permissions

GitHub runner status needs admin/read access to the configured orgs or repositories. Workflow jobs need Actions read access. Codeberg is queried through Forgejo-style API endpoints; if the instance does not expose the needed Actions API, the dashboard reports that source as unavailable while leaving the rest of the dashboard running.

## Tailscale Actions

Device status comes from:

```sh
tailscale status --json
```

SSH opens a local terminal using the configured `sshCommandTemplate`. On Linux the app tries `x-terminal-emulator`, `gnome-terminal`, `kgx`, `konsole`, then `xterm`. Restart is available only for devices with `restartable: true` in `dashboard.config.json`, asks for UI confirmation, then runs `restartCommandTemplate`.
