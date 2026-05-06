use chrono::Utc;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::Stdio,
};
use tauri::{AppHandle, Manager};
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
enum DashboardError {
    #[error("{0}")]
    Message(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Config parse error: {0}")]
    Config(#[from] serde_json::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
}

impl serde::Serialize for DashboardError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

type Result<T> = std::result::Result<T, DashboardError>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardConfig {
    #[serde(default = "default_refresh_interval")]
    refresh_interval_seconds: u64,
    #[serde(default)]
    github: GitHubConfig,
    #[serde(default)]
    forgejo: ForgejoConfig,
    #[serde(default)]
    tailscale: TailscaleConfig,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: default_refresh_interval(),
            github: GitHubConfig::default(),
            forgejo: ForgejoConfig::default(),
            tailscale: TailscaleConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GitHubConfig {
    #[serde(default = "default_github_token_env")]
    token_env: String,
    #[serde(default)]
    organizations: Vec<GitHubOrganization>,
    #[serde(default)]
    repositories: Vec<RepositoryRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubOrganization {
    owner: String,
    #[serde(default)]
    repositories: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ForgejoConfig {
    #[serde(default = "default_forgejo_token_env")]
    token_env: String,
    #[serde(default)]
    instances: Vec<ForgejoInstance>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForgejoInstance {
    name: String,
    base_url: String,
    #[serde(default)]
    token_env: Option<String>,
    #[serde(default)]
    repositories: Vec<RepositoryRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryRef {
    owner: String,
    repo: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TailscaleConfig {
    #[serde(default = "default_ssh_user")]
    default_ssh_user: String,
    #[serde(default = "default_ssh_template")]
    ssh_command_template: String,
    #[serde(default = "default_restart_template")]
    restart_command_template: String,
    #[serde(default)]
    devices: Vec<ConfiguredDevice>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfiguredDevice {
    name: String,
    ssh_user: Option<String>,
    #[serde(default)]
    restartable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardData {
    updated_at: String,
    refresh_interval_seconds: u64,
    sources: Vec<SourceStatus>,
    runners: Vec<Runner>,
    current_jobs: Vec<Job>,
    queued_jobs: Vec<Job>,
    devices: Vec<TailscaleDevice>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceStatus {
    provider: String,
    name: String,
    health: String,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Runner {
    id: String,
    provider: String,
    source: String,
    name: String,
    os: Option<String>,
    status: String,
    busy: bool,
    labels: Vec<String>,
    current_job_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    provider: String,
    source: String,
    repository: String,
    workflow: String,
    name: String,
    status: String,
    runner_id: Option<String>,
    runner_name: Option<String>,
    branch: Option<String>,
    sha: Option<String>,
    url: Option<String>,
    started_at: Option<String>,
    queued_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TailscaleDevice {
    id: String,
    name: String,
    host_name: String,
    dns_name: Option<String>,
    tailnet_ip: Option<String>,
    os: Option<String>,
    online: bool,
    last_seen: Option<String>,
    user: Option<String>,
    tags: Vec<String>,
    ssh_user: Option<String>,
    restartable: bool,
}

#[derive(Debug, Serialize)]
struct ActionResult {
    ok: bool,
    message: String,
}

#[tauri::command]
async fn get_dashboard_data(app: AppHandle) -> Result<DashboardData> {
    let config = load_config(&app)?;
    let mut sources = Vec::new();
    let mut runners = Vec::new();
    let mut current_jobs = Vec::new();
    let mut queued_jobs = Vec::new();

    poll_github(&config, &mut sources, &mut runners, &mut current_jobs, &mut queued_jobs).await;
    poll_forgejo(&config, &mut sources, &mut current_jobs, &mut queued_jobs).await;
    let devices = poll_tailscale(&config, &mut sources).await;
    attach_current_jobs(&mut runners, &current_jobs);

    Ok(DashboardData {
        updated_at: Utc::now().to_rfc3339(),
        refresh_interval_seconds: config.refresh_interval_seconds,
        sources,
        runners,
        current_jobs,
        queued_jobs,
        devices,
    })
}

#[tauri::command]
async fn open_ssh(app: AppHandle, device_name: String) -> Result<ActionResult> {
    let config = load_config(&app)?;
    let device = configured_device(&config, &device_name)?;
    let user = device
        .ssh_user
        .clone()
        .unwrap_or_else(|| config.tailscale.default_ssh_user.clone());
    let command = render_command(&config.tailscale.ssh_command_template, &user, &device.name);
    open_terminal(&command).await?;
    Ok(ActionResult {
        ok: true,
        message: format!("Opening SSH session for {}", device.name),
    })
}

#[tauri::command]
async fn restart_device(app: AppHandle, device_name: String) -> Result<ActionResult> {
    let config = load_config(&app)?;
    let device = configured_device(&config, &device_name)?;
    if !device.restartable {
        return Err(DashboardError::Message(format!(
            "{} is not restartable in dashboard.config.json",
            device.name
        )));
    }
    let user = device
        .ssh_user
        .clone()
        .unwrap_or_else(|| config.tailscale.default_ssh_user.clone());
    let command = render_command(&config.tailscale.restart_command_template, &user, &device.name);
    run_shell_command(&command).await?;
    Ok(ActionResult {
        ok: true,
        message: format!("Restart command sent to {}", device.name),
    })
}

async fn poll_github(
    config: &DashboardConfig,
    sources: &mut Vec<SourceStatus>,
    runners: &mut Vec<Runner>,
    current_jobs: &mut Vec<Job>,
    queued_jobs: &mut Vec<Job>,
) {
    if config.github.organizations.is_empty() && config.github.repositories.is_empty() {
        return;
    }

    let Some(token) = token_from_env(&config.github.token_env) else {
        sources.push(source_error("github", "GitHub", format!("Missing {}", config.github.token_env)));
        return;
    };

    let client = reqwest::Client::new();
    for org in &config.github.organizations {
        let source = format!("org/{}", org.owner);
        match fetch_github_org_runners(&client, &token, &org.owner).await {
            Ok(mut next) => {
                runners.append(&mut next);
                sources.push(source_ok("github", &source));
            }
            Err(err) => sources.push(source_error("github", &source, err.to_string())),
        }

        for repo in &org.repositories {
            let repo_ref = RepositoryRef {
                owner: org.owner.clone(),
                repo: repo.clone(),
            };
            poll_github_repo_jobs(&client, &token, &repo_ref, current_jobs, queued_jobs, sources).await;
        }
    }

    for repo in &config.github.repositories {
        let source = format!("{}/{}", repo.owner, repo.repo);
        match fetch_github_repo_runners(&client, &token, repo).await {
            Ok(mut next) => {
                runners.append(&mut next);
                sources.push(source_ok("github", &source));
            }
            Err(err) => sources.push(source_error("github", &source, err.to_string())),
        }
        poll_github_repo_jobs(&client, &token, repo, current_jobs, queued_jobs, sources).await;
    }
}

async fn poll_github_repo_jobs(
    client: &reqwest::Client,
    token: &str,
    repo: &RepositoryRef,
    current_jobs: &mut Vec<Job>,
    queued_jobs: &mut Vec<Job>,
    sources: &mut Vec<SourceStatus>,
) {
    let source = format!("{}/{}", repo.owner, repo.repo);
    match fetch_github_jobs(client, token, repo).await {
        Ok(jobs) => split_jobs(jobs, current_jobs, queued_jobs),
        Err(err) => sources.push(source_error("github", &source, format!("Jobs unavailable: {err}"))),
    }
}

async fn poll_forgejo(
    config: &DashboardConfig,
    sources: &mut Vec<SourceStatus>,
    current_jobs: &mut Vec<Job>,
    queued_jobs: &mut Vec<Job>,
) {
    if config.forgejo.instances.is_empty() {
        return;
    }

    let client = reqwest::Client::new();
    for instance in &config.forgejo.instances {
        let token_env = instance.token_env.as_ref().unwrap_or(&config.forgejo.token_env);
        let token = token_from_env(token_env);
        for repo in &instance.repositories {
            let source = format!("{} {}/{}", instance.name, repo.owner, repo.repo);
            match fetch_forgejo_jobs(&client, instance, token.as_deref(), repo).await {
                Ok(jobs) => {
                    split_jobs(jobs, current_jobs, queued_jobs);
                    sources.push(source_ok("forgejo", &source));
                }
                Err(err) => sources.push(source_error(
                    "forgejo",
                    &source,
                    format!("API unsupported, permission missing, or unavailable: {err}"),
                )),
            }
        }
    }
}

async fn poll_tailscale(config: &DashboardConfig, sources: &mut Vec<SourceStatus>) -> Vec<TailscaleDevice> {
    match Command::new("tailscale")
        .arg("status")
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(output) if output.status.success() => match serde_json::from_slice::<Value>(&output.stdout) {
            Ok(json) => {
                sources.push(source_ok("tailscale", "Tailscale"));
                normalize_tailscale_devices(&json, &config.tailscale.devices)
            }
            Err(err) => {
                sources.push(source_error("tailscale", "Tailscale", format!("Invalid JSON: {err}")));
                Vec::new()
            }
        },
        Ok(output) => {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            sources.push(source_error("tailscale", "Tailscale", if message.is_empty() { "tailscale status failed".into() } else { message }));
            Vec::new()
        }
        Err(err) => {
            sources.push(source_error("tailscale", "Tailscale", err.to_string()));
            Vec::new()
        }
    }
}

async fn fetch_github_org_runners(client: &reqwest::Client, token: &str, owner: &str) -> Result<Vec<Runner>> {
    let url = format!("https://api.github.com/orgs/{owner}/actions/runners?per_page=100");
    let body = github_get(client, token, &url).await?;
    Ok(normalize_github_runners(&format!("org/{owner}"), body["runners"].as_array().cloned().unwrap_or_default()))
}

async fn fetch_github_repo_runners(client: &reqwest::Client, token: &str, repo: &RepositoryRef) -> Result<Vec<Runner>> {
    let source = format!("{}/{}", repo.owner, repo.repo);
    let url = format!("https://api.github.com/repos/{}/{}/actions/runners?per_page=100", repo.owner, repo.repo);
    let body = github_get(client, token, &url).await?;
    Ok(normalize_github_runners(&source, body["runners"].as_array().cloned().unwrap_or_default()))
}

async fn fetch_github_jobs(client: &reqwest::Client, token: &str, repo: &RepositoryRef) -> Result<Vec<Job>> {
    let runs_url = format!(
        "https://api.github.com/repos/{}/{}/actions/runs?status=queued&per_page=50",
        repo.owner, repo.repo
    );
    let queued_runs = github_get(client, token, &runs_url).await?;
    let in_progress_url = format!(
        "https://api.github.com/repos/{}/{}/actions/runs?status=in_progress&per_page=50",
        repo.owner, repo.repo
    );
    let active_runs = github_get(client, token, &in_progress_url).await?;

    let mut jobs = Vec::new();
    for run in queued_runs["workflow_runs"].as_array().into_iter().flatten().chain(active_runs["workflow_runs"].as_array().into_iter().flatten()) {
        if let Some(run_id) = run["id"].as_i64() {
            let url = format!("https://api.github.com/repos/{}/{}/actions/runs/{run_id}/jobs?filter=latest&per_page=100", repo.owner, repo.repo);
            let jobs_body = github_get(client, token, &url).await?;
            for job in jobs_body["jobs"].as_array().into_iter().flatten() {
                if let Some(normalized) = normalize_github_job(&format!("{}/{}", repo.owner, repo.repo), job) {
                    jobs.push(normalized);
                }
            }
        }
    }
    Ok(jobs)
}

async fn fetch_forgejo_jobs(
    client: &reqwest::Client,
    instance: &ForgejoInstance,
    token: Option<&str>,
    repo: &RepositoryRef,
) -> Result<Vec<Job>> {
    let base = instance.base_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/repos/{}/{}/actions/runs?limit=50", repo.owner, repo.repo);
    let mut request = client.get(url).header(USER_AGENT, "txst-lab-dashboard").header(ACCEPT, "application/json");
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("token {token}"));
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        return Err(DashboardError::Message(format!("HTTP {}", response.status())));
    }
    let body: Value = response.json().await?;
    let runs = body["workflow_runs"]
        .as_array()
        .or_else(|| body["runs"].as_array())
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default();

    let repository = format!("{}/{}", repo.owner, repo.repo);
    let mut jobs = Vec::new();
    for run in runs {
        if let Some(job) = normalize_forgejo_job(&instance.name, &repository, &run) {
            jobs.push(job);
        }
    }
    Ok(jobs)
}

async fn github_get(client: &reqwest::Client, token: &str, url: &str) -> Result<Value> {
    let response = client
        .get(url)
        .header(USER_AGENT, "txst-lab-dashboard")
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .bearer_auth(token)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(DashboardError::Message(format!("HTTP {}", response.status())));
    }
    Ok(response.json().await?)
}

fn normalize_github_runners(source: &str, runners: Vec<Value>) -> Vec<Runner> {
    runners
        .into_iter()
        .map(|runner| {
            let id = value_to_string(&runner["id"]).unwrap_or_else(|| "unknown".into());
            let labels = runner["labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|label| label["name"].as_str().map(ToOwned::to_owned))
                .collect();
            Runner {
                id: format!("github:{source}:{id}"),
                provider: "github".into(),
                source: source.into(),
                name: runner["name"].as_str().unwrap_or("Unknown runner").into(),
                os: runner["os"].as_str().map(ToOwned::to_owned),
                status: runner["status"].as_str().unwrap_or("unknown").into(),
                busy: runner["busy"].as_bool().unwrap_or(false),
                labels,
                current_job_id: None,
            }
        })
        .collect()
}

fn normalize_github_job(repository: &str, job: &Value) -> Option<Job> {
    let status = normalize_job_status(job["status"].as_str().unwrap_or("unknown"));
    if !is_active_job_status(&status) {
        return None;
    }
    let runner_id = value_to_string(&job["runner_id"]).map(|id| format!("github:{repository}:{id}"));
    Some(Job {
        id: format!("github:{repository}:{}", value_to_string(&job["id"]).unwrap_or_else(|| "unknown".into())),
        provider: "github".into(),
        source: repository.into(),
        repository: repository.into(),
        workflow: job["workflow_name"].as_str().unwrap_or("Workflow").into(),
        name: job["name"].as_str().unwrap_or("Job").into(),
        status,
        runner_id,
        runner_name: job["runner_name"].as_str().map(ToOwned::to_owned),
        branch: job["head_branch"].as_str().map(ToOwned::to_owned),
        sha: job["head_sha"].as_str().map(short_sha),
        url: job["html_url"].as_str().map(ToOwned::to_owned),
        started_at: job["started_at"].as_str().map(ToOwned::to_owned),
        queued_at: job["created_at"].as_str().map(ToOwned::to_owned),
    })
}

fn normalize_forgejo_job(source: &str, repository: &str, run: &Value) -> Option<Job> {
    let status = normalize_job_status(
        run["status"]
            .as_str()
            .or_else(|| run["state"].as_str())
            .unwrap_or("unknown"),
    );
    if !is_active_job_status(&status) {
        return None;
    }
    let id = value_to_string(&run["id"])
        .or_else(|| value_to_string(&run["run_id"]))
        .unwrap_or_else(|| "unknown".into());
    Some(Job {
        id: format!("forgejo:{repository}:{id}"),
        provider: "forgejo".into(),
        source: source.into(),
        repository: repository.into(),
        workflow: run["workflow_name"].as_str().or_else(|| run["name"].as_str()).unwrap_or("Workflow").into(),
        name: run["display_title"].as_str().or_else(|| run["job_name"].as_str()).or_else(|| run["name"].as_str()).unwrap_or("Job").into(),
        status,
        runner_id: value_to_string(&run["runner_id"]),
        runner_name: run["runner_name"].as_str().map(ToOwned::to_owned),
        branch: run["head_branch"].as_str().or_else(|| run["ref_name"].as_str()).map(ToOwned::to_owned),
        sha: run["head_sha"].as_str().or_else(|| run["commit_sha"].as_str()).map(short_sha),
        url: run["html_url"].as_str().or_else(|| run["url"].as_str()).map(ToOwned::to_owned),
        started_at: run["started_at"].as_str().map(ToOwned::to_owned),
        queued_at: run["created_at"].as_str().map(ToOwned::to_owned),
    })
}

fn normalize_tailscale_devices(json: &Value, configured: &[ConfiguredDevice]) -> Vec<TailscaleDevice> {
    let configured_by_name: HashMap<&str, &ConfiguredDevice> = configured.iter().map(|device| (device.name.as_str(), device)).collect();
    let mut peers = Vec::new();
    if let Some(self_node) = json["Self"].as_object() {
        peers.push(Value::Object(self_node.clone()));
    }
    if let Some(peer_map) = json["Peer"].as_object() {
        peers.extend(peer_map.values().cloned());
    }

    let mut devices: Vec<_> = peers
        .iter()
        .map(|peer| {
            let host_name = peer["HostName"].as_str().or_else(|| peer["DNSName"].as_str()).unwrap_or("unknown").to_string();
            let dns_name = peer["DNSName"].as_str().map(ToOwned::to_owned);
            let name = dns_name
                .as_deref()
                .map(trim_tailnet_name)
                .unwrap_or_else(|| host_name.clone());
            let configured = configured_by_name.get(name.as_str()).or_else(|| configured_by_name.get(host_name.as_str())).copied();
            let tailnet_ip = peer["TailscaleIPs"].as_array().and_then(|ips| ips.first()).and_then(Value::as_str).map(ToOwned::to_owned);
            let tags = peer["Tags"].as_array().into_iter().flatten().filter_map(|tag| tag.as_str().map(ToOwned::to_owned)).collect();
            TailscaleDevice {
                id: peer["ID"].as_str().or_else(|| peer["PublicKey"].as_str()).unwrap_or(&name).to_string(),
                name,
                host_name,
                dns_name,
                tailnet_ip,
                os: peer["OS"].as_str().map(ToOwned::to_owned),
                online: peer["Online"].as_bool().unwrap_or(false),
                last_seen: peer["LastSeen"].as_str().map(ToOwned::to_owned),
                user: peer["User"].as_str().map(ToOwned::to_owned),
                tags,
                ssh_user: configured.and_then(|device| device.ssh_user.clone()),
                restartable: configured.map(|device| device.restartable).unwrap_or(false),
            }
        })
        .collect();
    devices.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.name.cmp(&b.name)));
    devices
}

fn split_jobs(jobs: Vec<Job>, current_jobs: &mut Vec<Job>, queued_jobs: &mut Vec<Job>) {
    for job in jobs {
        if job.status == "in_progress" || job.status == "running" {
            current_jobs.push(job);
        } else {
            queued_jobs.push(job);
        }
    }
}

fn attach_current_jobs(runners: &mut [Runner], current_jobs: &[Job]) {
    for runner in runners {
        runner.current_job_id = current_jobs
            .iter()
            .find(|job| job.runner_id.as_deref() == Some(runner.id.as_str()) || job.runner_name.as_deref() == Some(runner.name.as_str()))
            .map(|job| job.id.clone());
    }
}

fn load_config(app: &AppHandle) -> Result<DashboardConfig> {
    let candidates = config_candidates(app);
    for candidate in candidates {
        if candidate.exists() {
            let text = fs::read_to_string(candidate)?;
            return Ok(serde_json::from_str(&text)?);
        }
    }
    Ok(DashboardConfig::default())
}

fn config_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(explicit_path) = env::var("TXST_DASHBOARD_CONFIG") {
        push_config_candidate(&mut candidates, PathBuf::from(explicit_path));
    }
    if let Ok(cwd) = env::current_dir() {
        push_config_candidate(&mut candidates, cwd.join("dashboard.config.json"));
    }
    if let Ok(appimage_path) = env::var("APPIMAGE") {
        push_config_candidate(&mut candidates, sibling_config_path(&PathBuf::from(appimage_path)));
    }
    if let Ok(exe_path) = env::current_exe() {
        push_config_candidate(&mut candidates, sibling_config_path(&exe_path));
    }
    if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
        push_config_candidate(
            &mut candidates,
            PathBuf::from(config_home).join("txst-lab-dashboard/dashboard.config.json"),
        );
    }
    if let Ok(home) = env::var("HOME") {
        push_config_candidate(
            &mut candidates,
            PathBuf::from(home).join(".config/txst-lab-dashboard/dashboard.config.json"),
        );
    }
    push_config_candidate(
        &mut candidates,
        PathBuf::from("/etc/txst-lab-dashboard/dashboard.config.json"),
    );
    if let Ok(resource) = app.path().resource_dir() {
        push_config_candidate(&mut candidates, resource.join("dashboard.config.json"));
    }

    dedupe_paths(candidates)
}

fn push_config_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }
    candidates.push(path);
}

fn sibling_config_path(path: &Path) -> PathBuf {
    path.parent()
        .map(|parent| parent.join("dashboard.config.json"))
        .unwrap_or_else(|| PathBuf::from("dashboard.config.json"))
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn configured_device<'a>(config: &'a DashboardConfig, device_name: &str) -> Result<&'a ConfiguredDevice> {
    config
        .tailscale
        .devices
        .iter()
        .find(|device| device.name == device_name)
        .ok_or_else(|| DashboardError::Message(format!("{device_name} is not configured for actions")))
}

fn render_command(template: &str, user: &str, host: &str) -> String {
    template.replace("{user}", user).replace("{host}", host)
}

async fn open_terminal(command: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("osascript")
            .arg("-e")
            .arg(format!("tell application \"Terminal\" to do script {}", json_string(command)))
            .status()
            .await?;
        if status.success() {
            return Ok(());
        }
        return Err(DashboardError::Message(format!("Terminal exited with {status}")));
    }

    #[cfg(target_os = "linux")]
    {
        open_linux_terminal(command).await
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "linux")))]
    {
        run_shell_command(command).await
    }
}

#[cfg(target_os = "linux")]
async fn open_linux_terminal(command: &str) -> Result<()> {
    let shell_command = format!("{command}; printf '\\nCommand finished. Press Enter to close.'; read _");
    let candidates: [(&str, &[&str]); 5] = [
        ("x-terminal-emulator", &["-e", "sh", "-lc"]),
        ("gnome-terminal", &["--", "sh", "-lc"]),
        ("kgx", &["--", "sh", "-lc"]),
        ("konsole", &["-e", "sh", "-lc"]),
        ("xterm", &["-e", "sh", "-lc"]),
    ];

    for (program, args) in candidates {
        let mut command_builder = Command::new(program);
        for arg in args {
            command_builder.arg(arg);
        }
        command_builder.arg(&shell_command);
        match command_builder.spawn() {
            Ok(_) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(DashboardError::Io(err)),
        }
    }

    Err(DashboardError::Message(
        "No supported terminal emulator found. Install x-terminal-emulator, gnome-terminal, kgx, konsole, xfce4-terminal, or xterm.".into(),
    ))
}

async fn run_shell_command(command: &str) -> Result<()> {
    let status = Command::new("sh").arg("-lc").arg(command).status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(DashboardError::Message(format!("Command exited with {status}")))
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

fn token_from_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn source_ok(provider: &str, name: &str) -> SourceStatus {
    SourceStatus {
        provider: provider.into(),
        name: name.into(),
        health: "ok".into(),
        message: None,
    }
}

fn source_error(provider: &str, name: &str, message: String) -> SourceStatus {
    SourceStatus {
        provider: provider.into(),
        name: name.into(),
        health: "error".into(),
        message: Some(message),
    }
}

fn normalize_job_status(status: &str) -> String {
    match status.to_ascii_lowercase().as_str() {
        "requested" | "queued" => "queued",
        "waiting" => "waiting",
        "pending" => "pending",
        "in-progress" | "in_progress" => "in_progress",
        "running" => "running",
        _ => "unknown",
    }
    .into()
}

fn is_active_job_status(status: &str) -> bool {
    matches!(status, "queued" | "waiting" | "pending" | "in_progress" | "running")
}

fn value_to_string(value: &Value) -> Option<String> {
    if value.is_null() {
        None
    } else if let Some(value) = value.as_str() {
        Some(value.to_string())
    } else {
        Some(value.to_string())
    }
}

fn short_sha(value: &str) -> String {
    value.chars().take(7).collect()
}

fn trim_tailnet_name(value: &str) -> String {
    value.trim_end_matches('.').split('.').next().unwrap_or(value).to_string()
}

fn default_refresh_interval() -> u64 {
    30
}

fn default_github_token_env() -> String {
    "GITHUB_TOKEN".into()
}

fn default_forgejo_token_env() -> String {
    "CODEBERG_TOKEN".into()
}

fn default_ssh_user() -> String {
    "lab".into()
}

fn default_ssh_template() -> String {
    "tailscale ssh {user}@{host}".into()
}

fn default_restart_template() -> String {
    "tailscale ssh {user}@{host} sudo /sbin/shutdown -r now".into()
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_dashboard_data, open_ssh, restart_device])
        .run(tauri::generate_context!())
        .expect("error while running TXST Lab Dashboard");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_requires_allowlisted_device() {
        let config = DashboardConfig {
            tailscale: TailscaleConfig {
                devices: vec![ConfiguredDevice {
                    name: "runner-01".into(),
                    ssh_user: Some("lab".into()),
                    restartable: true,
                }],
                ..TailscaleConfig::default()
            },
            ..DashboardConfig::default()
        };

        assert!(configured_device(&config, "runner-01").is_ok());
        assert!(configured_device(&config, "runner-02").is_err());
    }

    #[test]
    fn renders_command_template() {
        assert_eq!(
            render_command("tailscale ssh {user}@{host} uptime", "lab", "runner-01"),
            "tailscale ssh lab@runner-01 uptime"
        );
    }

    #[test]
    fn normalizes_tailscale_devices() {
        let json: Value = serde_json::json!({
            "Self": { "ID": "self", "HostName": "dashboard", "Online": true, "TailscaleIPs": ["100.64.0.1"] },
            "Peer": {
                "abc": {
                    "ID": "abc",
                    "HostName": "lab-runner-01",
                    "DNSName": "lab-runner-01.example.ts.net.",
                    "Online": false,
                    "OS": "linux"
                }
            }
        });
        let devices = normalize_tailscale_devices(
            &json,
            &[ConfiguredDevice {
                name: "lab-runner-01".into(),
                ssh_user: Some("lab".into()),
                restartable: true,
            }],
        );

        assert_eq!(devices.len(), 2);
        assert!(devices.iter().any(|device| device.name == "lab-runner-01" && device.restartable));
    }
}
