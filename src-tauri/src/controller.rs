use std::{
    collections::HashSet,
    fs,
    net::TcpListener,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

use crate::{
    electron_wco::{launch_with_transparent_wco, WcoLaunch},
    injector::{native_titlebar_bridge_ready, read_browser_identity, InjectorEngine},
    managed_launch::{
        snapshot_executable_processes, snapshot_executable_processes_strict, Observation,
        ProcessKey, ProcessRecord, ProcessTracker, WatcherAction, MSG_AUTO_APPLIED,
        MSG_DEBUG_TIMEOUT, MSG_EXISTING, MSG_SUSPENDED, MSG_TAKING_OVER, MSG_WAITING,
    },
    models::RuntimeStatus,
    payload::ActivePayload,
    settings::write_json_transaction,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PREFERRED_DEBUG_PORT: u16 = 9337;
const RUNTIME_SCHEMA_VERSION: u8 = 2;
/// 强制结束后先同步等这么久；多数情况下 Grok 在此期间就已彻底退出。
const EXIT_QUICK_WAIT: Duration = Duration::from_secs(15);
/// 官方外观重启后观察这么久，确认新进程没有因单实例锁被放弃。
const OFFICIAL_LAUNCH_SETTLE: Duration = Duration::from_secs(20);
const RELAUNCH_RETRY_INTERVAL: Duration = Duration::from_secs(3);
const RELAUNCH_MAX_ATTEMPTS: u8 = 10;
/// 等旧进程退出的总时长上限；超过后报错，避免无限期停在"等待退出"。
const RELAUNCH_MAX_WAIT: Duration = Duration::from_secs(600);

pub const MSG_WAIT_EXIT_MANAGED: &str = "等待旧的 Grok 进程完全退出，随后自动重启并应用背景";
pub const MSG_WAIT_EXIT_OFFICIAL: &str = "等待旧的 Grok 进程完全退出，随后自动以官方外观重启";
const MSG_RELAUNCH_FAILED: &str =
    "旧的 Grok 进程长时间未释放，自动重启多次失败，请稍后手动启动 Grok。";

/// 强制结束后 Grok（Electron）进程可能在进程表中滞留数十秒到数分钟，期间仍占着
/// 单实例锁、严格快照也无法确认其身份。这时不报错放弃，而是登记待重启，由 watcher
/// 在旧进程真正消失后重新启动，保证被插件关掉的 Grok 一定会被拉起来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelaunchKind {
    /// 带调试端口与透明标题栏补丁重启并注入背景。
    Managed,
    /// 以官方外观（无调试参数）重启。
    Official,
}

struct RelaunchPlan {
    kind: RelaunchKind,
    /// 登记时仍滞留的旧进程；出现不在其中的新进程说明用户已自行启动了 Grok。
    lingering: HashSet<ProcessKey>,
    /// 本插件为重启而拉起的主进程 PID，不能被当成"用户自行启动"。
    own_pids: HashSet<u32>,
    attempts: u8,
    not_before: Instant,
    /// 首次登记待重启的时间，重试时沿用。
    since: Instant,
    /// 官方外观重启后正在观察的子进程：若它很快自行退出，需重试。
    watch: Option<(Child, Instant)>,
}

pub enum RelaunchPoll {
    Waiting,
    Due,
    Superseded(RelaunchKind),
}

pub enum RelaunchOutcome {
    Attached,
    Official,
    Retrying,
    Idle,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokInstall {
    package_root: String,
    executable: String,
    version: String,
    package_full_name: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeState {
    schema_version: u8,
    port: u16,
    browser_id: String,
    wco_enabled: bool,
    package_full_name: String,
    executable: String,
    created_at: String,
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn encoded_powershell(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    STANDARD.encode(bytes)
}

fn run_powershell(script: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded_powershell(script),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| error.to_string())?;
    if child
        .wait_timeout(timeout)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err("PowerShell 操作超时。".to_string());
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            if stdout.is_empty() {
                "PowerShell 操作失败。".to_string()
            } else {
                stdout
            }
        } else {
            stderr
        })
    }
}

fn normalized_path(path: &str) -> String {
    Path::new(path)
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn candidate_executables() -> Vec<PathBuf> {
    vec![PathBuf::from(crate::fuse_guard::TARGET)]
}

fn read_version(executable: &Path) -> String {
    let version_file = executable
        .parent()
        .map(|parent| parent.join("version"))
        .filter(|path| path.is_file());
    if let Some(path) = version_file {
        if let Ok(content) = fs::read_to_string(path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() && trimmed.len() <= 64 {
                return trimmed.to_string();
            }
        }
    }
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$info = [Diagnostics.FileVersionInfo]::GetVersionInfo({})
$version = "$($info.ProductVersion)".Trim()
if (-not $version) {{ $version = "$($info.FileVersion)".Trim() }}
if (-not $version) {{ throw '无法读取 Grok 版本。' }}
$version
"#,
        powershell_quote(&executable.to_string_lossy())
    );
    run_powershell(&script, Duration::from_secs(15)).unwrap_or_else(|_| "unknown".to_string())
}

fn discover_grok() -> Result<GrokInstall, String> {
    for executable in candidate_executables() {
        if !executable.is_file() {
            continue;
        }
        let package_root = executable
            .parent()
            .ok_or_else(|| "Grok 安装路径无效。".to_string())?
            .to_string_lossy()
            .to_string();
        let version = read_version(&executable);
        let package_full_name = format!("Grok.Desktop@{version}");
        let install = GrokInstall {
            package_root,
            executable: executable.to_string_lossy().to_string(),
            version,
            package_full_name,
        };
        let root = format!(
            "{}\\",
            normalized_path(&install.package_root).trim_end_matches('\\')
        );
        if !normalized_path(&install.executable).starts_with(&root) {
            continue;
        }
        if !normalized_path(&install.executable).ends_with("\\grok bot.exe") {
            continue;
        }
        return Ok(install);
    }
    Err("未找到 Grok Bot 桌面应用（预期路径：D:\\grok_bot\\Grok Bot\\Grok Bot.exe）。".to_string())
}

fn process_records_for(install: &GrokInstall) -> Result<Vec<ProcessRecord>, String> {
    snapshot_executable_processes(&install.executable)
}

fn process_ids_for(install: &GrokInstall) -> Result<Vec<u32>, String> {
    Ok(process_records_for(install)?
        .into_iter()
        .map(|process| process.key.pid)
        .collect())
}

fn debug_ports_for(install: &GrokInstall) -> Result<Vec<u16>, String> {
    let mut ports = process_records_for(install)?
        .into_iter()
        .filter_map(|process| process.debug_port)
        .collect::<Vec<_>>();
    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

fn is_disconnected_error(error: &str) -> bool {
    error.contains("CDP 会话已关闭")
        || error.contains("CDP 浏览器身份已变化")
        || error.contains("CDP 命令超时")
        || error.contains("CDP 连接超时")
        || error.contains("CDP 返回 HTTP")
}

fn session_has_wco(port: u16, browser_id: &str) -> bool {
    native_titlebar_bridge_ready(port, browser_id).unwrap_or(false)
}

/// 与恢复/修改可执行文件前相同的严格判定：任何同名进程无法确认身份都视为未退出；
fn grok_fully_exited(install: &GrokInstall) -> bool {
    matches!(
        snapshot_executable_processes_strict(&install.executable),
        Ok(processes) if processes.is_empty()
    )
}

fn lingering_keys(install: &GrokInstall) -> HashSet<ProcessKey> {
    snapshot_executable_processes(&install.executable)
        .map(|processes| processes.into_iter().map(|process| process.key).collect())
        .unwrap_or_default()
}

/// 结束所有路径匹配的 Grok 进程。返回 `false` 表示快速等待窗口内仍未彻底退出，
/// 调用方须登记待重启而不是立即启动。
fn stop_verified_grok(install: &GrokInstall) -> Result<bool, String> {
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$target = {}
$ProgressPreference = 'SilentlyContinue'
$processes = @(Get-CimInstance Win32_Process -Filter "Name='Grok Bot.exe'" | Where-Object {{
  $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).Equals($target, [StringComparison]::OrdinalIgnoreCase)
}})
foreach ($item in $processes) {{ Stop-Process -Id ([int]$item.ProcessId) -Force -ErrorAction SilentlyContinue }}
# Electron children may exit with their parent before the last Stop-Process.
# The caller independently verifies that all processes actually exited.
exit 0
"#,
        powershell_quote(&normalized_path(&install.executable))
    );
    run_powershell(&script, Duration::from_secs(30))?;
    let deadline = Instant::now() + EXIT_QUICK_WAIT;
    while !grok_fully_exited(install) {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(300));
    }
    Ok(true)
}

fn launch_grok(install: &GrokInstall, arguments: &[String]) -> Result<Child, String> {
    let executable = PathBuf::from(&install.executable);
    if !executable.is_file() {
        return Err("Grok 可执行文件不存在。".to_string());
    }
    Command::new(&executable)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("启动 Grok 失败：{error}"))
}

fn select_port(preferred: u16) -> Result<u16, String> {
    for port in preferred..=preferred.saturating_add(100) {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err("无法为 Grok 分配本机调试端口。".to_string())
}

pub struct ManagedProbe {
    pub processes: Vec<ProcessRecord>,
    pub has_ready_debug_session: bool,
    pub engine_alive: bool,
}

impl ManagedProbe {
    pub fn observation(&self) -> Observation {
        Observation {
            processes: self.processes.clone(),
            has_ready_debug_session: self.has_ready_debug_session,
            engine_alive: self.engine_alive,
            now: std::time::Instant::now(),
        }
    }
}

pub struct GrokController {
    state_path: PathBuf,
    engine: Option<InjectorEngine>,
    state: Option<RuntimeState>,
    status: RuntimeStatus,
    install_cache: Option<GrokInstall>,
    tracker: ProcessTracker,
    watcher_paused: bool,
    relaunch: Option<RelaunchPlan>,
}

impl GrokController {
    pub fn load(data_directory: &Path) -> Self {
        let state_path = data_directory.join("runtime.json");
        let state = fs::read_to_string(&state_path)
            .ok()
            .and_then(|content| serde_json::from_str::<RuntimeState>(&content).ok())
            .filter(|state| {
                state.schema_version == RUNTIME_SCHEMA_VERSION
                    && state.wco_enabled
                    && !state.browser_id.is_empty()
            });
        Self {
            state_path,
            engine: None,
            state,
            status: RuntimeStatus::default(),
            install_cache: None,
            tracker: ProcessTracker::new(),
            watcher_paused: false,
            relaunch: None,
        }
    }

    pub fn status(&self) -> RuntimeStatus {
        let mut status = self.status.clone();
        if let Some(engine) = &self.engine {
            status.active_targets = engine.active_targets();
        }
        status
    }

    pub fn watcher_paused(&self) -> bool {
        self.watcher_paused
    }

    pub fn set_watcher_paused(&mut self, paused: bool) {
        self.watcher_paused = paused;
    }

    pub fn set_managed_status(&mut self, phase: &str, message: &str) {
        self.status.phase = phase.to_string();
        self.status.message = message.to_string();
        if phase != "error" {
            self.status.last_error = None;
        }
    }

    pub fn set_managed_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.status.phase = "error".to_string();
        self.status.message = error.clone();
        self.status.last_error = Some(error);
    }

    fn cached_install(&mut self) -> Result<GrokInstall, String> {
        if let Some(install) = &self.install_cache {
            if Path::new(&install.executable).is_file() {
                return Ok(install.clone());
            }
            self.install_cache = None;
        }
        let install = discover_grok()?;
        self.install_cache = Some(install.clone());
        Ok(install)
    }

    fn saved_session_is_live(&self, install: &GrokInstall) -> bool {
        let Some(state) = &self.state else {
            return false;
        };
        state.package_full_name == install.package_full_name
            && state.wco_enabled
            && normalized_path(&state.executable) == normalized_path(&install.executable)
            && read_browser_identity(state.port).ok().as_deref() == Some(state.browser_id.as_str())
            && session_has_wco(state.port, &state.browser_id)
    }

    fn live_debug_session_ready(processes: &[ProcessRecord]) -> bool {
        for process in processes {
            let Some(port) = process.debug_port else {
                continue;
            };
            let Ok(browser_id) = read_browser_identity(port) else {
                continue;
            };
            if session_has_wco(port, &browser_id) {
                return true;
            }
        }
        false
    }

    pub fn probe_managed(&mut self) -> Result<ManagedProbe, String> {
        if let Some(error) = crate::fuse_guard::recovery_error() {
            return Err(error);
        }
        let install = self.cached_install()?;
        let processes = self.tracker.scan(&install.executable)?;
        let engine_alive = self
            .engine
            .as_ref()
            .is_some_and(InjectorEngine::is_connected);
        let has_ready_debug_session = engine_alive
            || self.saved_session_is_live(&install)
            || Self::live_debug_session_ready(&processes);
        self.status.grok_version = Some(install.version);
        Ok(ManagedProbe {
            processes,
            has_ready_debug_session,
            engine_alive,
        })
    }

    pub fn apply_watcher_action_status(&mut self, action: WatcherAction) {
        match action {
            WatcherAction::Wait => {
                if !matches!(self.status.phase.as_str(), "active" | "paused" | "error") {
                    self.set_managed_status("idle", MSG_WAITING);
                }
            }
            WatcherAction::ReleaseAndWait => {
                self.set_managed_status("idle", MSG_WAITING);
            }
            WatcherAction::ReportExistingUnmanaged => {
                self.set_managed_status("idle", MSG_EXISTING);
            }
            WatcherAction::WaitForDebugPort | WatcherAction::Takeover | WatcherAction::Attach => {
                self.set_managed_status("starting", MSG_TAKING_OVER);
            }
            WatcherAction::ReportDebugTimeout => {
                self.set_managed_error(MSG_DEBUG_TIMEOUT);
            }
            WatcherAction::KeepActive | WatcherAction::Suspend => {}
        }
    }

    pub fn auto_takeover(&mut self, payload: ActivePayload) -> Result<RuntimeStatus, String> {
        self.set_managed_status("starting", MSG_TAKING_OVER);
        self.apply(payload, true)?;
        if self.status.phase == "active" {
            self.set_managed_status("active", MSG_AUTO_APPLIED);
        }
        Ok(self.status())
    }

    pub fn attach_without_restart(
        &mut self,
        payload: ActivePayload,
    ) -> Result<RuntimeStatus, String> {
        self.set_managed_status("starting", MSG_TAKING_OVER);
        self.apply(payload, false)?;
        if self.status.phase == "active" {
            self.set_managed_status("active", MSG_AUTO_APPLIED);
        }
        Ok(self.status())
    }

    pub fn release_stale_session(&mut self) -> Result<RuntimeStatus, String> {
        self.engine = None;
        self.write_state(None)?;
        self.status.active_targets = 0;
        self.set_managed_status("idle", MSG_WAITING);
        Ok(self.status())
    }

    fn drop_disconnected_engine(&mut self) {
        if self
            .engine
            .as_ref()
            .is_some_and(|engine| !engine.is_connected())
        {
            self.engine = None;
        }
    }

    pub fn relaunch_pending(&self) -> bool {
        self.relaunch.is_some()
    }

    /// 暂停（含恢复官方外观）期间不再注入，待重启统一按官方外观执行。
    fn effective_relaunch_kind(&self, kind: RelaunchKind) -> RelaunchKind {
        if self.watcher_paused {
            RelaunchKind::Official
        } else {
            kind
        }
    }

    fn show_relaunch_wait(&mut self, kind: RelaunchKind) {
        let (phase, message) = match self.effective_relaunch_kind(kind) {
            RelaunchKind::Managed => ("starting", MSG_WAIT_EXIT_MANAGED),
            RelaunchKind::Official => ("restoring", MSG_WAIT_EXIT_OFFICIAL),
        };
        self.set_managed_status(phase, message);
        self.status.active_targets = 0;
    }

    fn fail_relaunch(&mut self, error: &str) {
        self.relaunch = None;
        self.set_managed_error(error);
    }

    fn schedule_relaunch(
        &mut self,
        install: &GrokInstall,
        kind: RelaunchKind,
        attempts: u8,
        own_pid: Option<u32>,
        delay: Duration,
    ) {
        let (mut own_pids, since) = self
            .relaunch
            .take()
            .map(|plan| (plan.own_pids, plan.since))
            .unwrap_or_else(|| (HashSet::new(), Instant::now()));
        own_pids.extend(own_pid);
        self.relaunch = Some(RelaunchPlan {
            kind,
            lingering: lingering_keys(install),
            own_pids,
            attempts,
            not_before: Instant::now() + delay,
            since,
            watch: None,
        });
        self.show_relaunch_wait(kind);
    }

    fn finish_official_relaunch(&mut self, kind: RelaunchKind) {
        if self.watcher_paused && kind == RelaunchKind::Managed {
            self.set_managed_status("paused", MSG_SUSPENDED);
        } else if kind == RelaunchKind::Official {
            self.set_managed_status("idle", "已恢复官方外观");
        } else {
            self.set_managed_status("idle", "已重新启动 Grok");
        }
        self.status.active_targets = 0;
    }

    /// 以官方外观启动（先还原临时安全开关），并登记观察新进程是否很快自行退出。
    fn launch_official_watched(
        &mut self,
        install: &GrokInstall,
        kind: RelaunchKind,
        attempts: u8,
    ) -> Result<(), String> {
        let child = {
            let _launch_guard = crate::fuse_guard::restore_before_official_launch()?;
            launch_grok(install, &[])?
        };
        let (mut own_pids, since) = self
            .relaunch
            .take()
            .map(|plan| (plan.own_pids, plan.since))
            .unwrap_or_else(|| (HashSet::new(), Instant::now()));
        own_pids.insert(child.id());
        self.relaunch = Some(RelaunchPlan {
            kind,
            lingering: HashSet::new(),
            own_pids,
            attempts,
            not_before: Instant::now(),
            since,
            watch: Some((child, Instant::now() + OFFICIAL_LAUNCH_SETTLE)),
        });
        self.finish_official_relaunch(kind);
        Ok(())
    }

    /// watcher 每个 tick 调用：判断旧进程是否已彻底退出、用户是否已自行启动 Grok。
    pub fn poll_relaunch(&mut self) -> Result<RelaunchPoll, String> {
        let install = self.cached_install()?;
        let Some(plan) = self.relaunch.as_mut() else {
            return Ok(RelaunchPoll::Waiting);
        };
        let kind = plan.kind;
        if let Some((child, deadline)) = plan.watch.as_mut() {
            if matches!(child.try_wait(), Ok(Some(_))) {
                let attempts = plan.attempts.saturating_add(1);
                if attempts >= RELAUNCH_MAX_ATTEMPTS {
                    self.fail_relaunch(MSG_RELAUNCH_FAILED);
                    return Err(MSG_RELAUNCH_FAILED.to_string());
                }
                self.schedule_relaunch(&install, kind, attempts, None, RELAUNCH_RETRY_INTERVAL);
            } else if Instant::now() >= *deadline {
                self.relaunch = None;
            }
            return Ok(RelaunchPoll::Waiting);
        }
        let records = snapshot_executable_processes(&install.executable)?;
        let user_launched = records.iter().any(|record| {
            !plan.lingering.contains(&record.key) && !plan.own_pids.contains(&record.key.pid)
        });
        if user_launched {
            self.relaunch = None;
            if self.effective_relaunch_kind(kind) == RelaunchKind::Official {
                self.finish_official_relaunch(kind);
            }
            return Ok(RelaunchPoll::Superseded(kind));
        }
        let not_before = plan.not_before;
        let since = plan.since;
        let due = Instant::now() >= not_before && grok_fully_exited(&install);
        if !due && since.elapsed() >= RELAUNCH_MAX_WAIT {
            self.fail_relaunch(MSG_RELAUNCH_FAILED);
            return Err(MSG_RELAUNCH_FAILED.to_string());
        }
        self.show_relaunch_wait(kind);
        Ok(if due {
            RelaunchPoll::Due
        } else {
            RelaunchPoll::Waiting
        })
    }

    /// 旧进程已彻底退出后由 watcher 调用：按待重启类型重新启动 Grok。
    pub fn run_pending_relaunch(
        &mut self,
        payload: Option<ActivePayload>,
    ) -> Result<RelaunchOutcome, String> {
        let Some(plan) = self.relaunch.as_ref() else {
            return Ok(RelaunchOutcome::Idle);
        };
        if plan.watch.is_some() {
            return Ok(RelaunchOutcome::Idle);
        }
        let kind = plan.kind;
        let attempts = plan.attempts;
        let install = match self.cached_install() {
            Ok(install) => install,
            Err(error) => {
                self.fail_relaunch(&error);
                return Err(error);
            }
        };
        if !grok_fully_exited(&install) {
            return Ok(RelaunchOutcome::Retrying);
        }
        let payload = match self.effective_relaunch_kind(kind) {
            RelaunchKind::Managed => payload,
            RelaunchKind::Official => None,
        };
        let Some(payload) = payload else {
            if let Err(error) = self.launch_official_watched(&install, kind, attempts) {
                return self.retry_after_launch_error(&install, kind, attempts, None, error, false);
            }
            return Ok(RelaunchOutcome::Official);
        };
        self.set_managed_status("starting", "正在重新启动 Grok");
        let launched = select_port(PREFERRED_DEBUG_PORT).and_then(|port| {
            launch_with_transparent_wco(Path::new(&install.executable), port)
                .map(|launch| (port, launch))
        });
        match launched {
            Ok((port, WcoLaunch::Ready(browser_id))) => {
                if let Err(error) = self.attach_launched(&install, port, browser_id, payload) {
                    self.fail_relaunch(&error);
                    return Err(error);
                }
                self.set_managed_status("active", MSG_AUTO_APPLIED);
                Ok(RelaunchOutcome::Attached)
            }
            Ok((_, WcoLaunch::ExitedEarly { pid })) => self.retry_after_launch_error(
                &install,
                kind,
                attempts,
                Some(pid),
                MSG_RELAUNCH_FAILED.to_string(),
                true,
            ),
            Err(error) => {
                self.retry_after_launch_error(&install, kind, attempts, None, error, false)
            }
        }
    }

    /// 被插件关掉的 Grok 必须重新拉起来，启动失败不能直接放弃：
    /// - 仍有滞留进程（含刚失败的这次启动留下的）时继续等待，不计入失败次数，
    ///   总时长由 `RELAUNCH_MAX_WAIT` 兜底；
    /// - 进程已清空仍失败，则计一次失败，间隔后重试，达到上限才报告最后一次错误。
    fn retry_after_launch_error(
        &mut self,
        install: &GrokInstall,
        kind: RelaunchKind,
        attempts: u8,
        own_pid: Option<u32>,
        error: String,
        always_count: bool,
    ) -> Result<RelaunchOutcome, String> {
        // 映像仍被占用（刚结束的进程或杀毒扫描）同属"尚未完全退出"，不计失败次数。
        let still_exiting =
            crate::fuse_guard::is_image_in_use(&error) || !grok_fully_exited(install);
        let attempts = if always_count || !still_exiting {
            attempts.saturating_add(1)
        } else {
            attempts
        };
        if attempts >= RELAUNCH_MAX_ATTEMPTS {
            let error = format!("{MSG_RELAUNCH_FAILED}（{error}）");
            self.fail_relaunch(&error);
            return Err(error);
        }
        self.schedule_relaunch(install, kind, attempts, own_pid, RELAUNCH_RETRY_INTERVAL);
        Ok(RelaunchOutcome::Retrying)
    }

    /// 记录新启动的调试会话并开始注入。
    fn attach_launched(
        &mut self,
        install: &GrokInstall,
        port: u16,
        browser_id: String,
        payload: ActivePayload,
    ) -> Result<(), String> {
        self.write_state(Some(RuntimeState {
            schema_version: RUNTIME_SCHEMA_VERSION,
            port,
            browser_id: browser_id.clone(),
            wco_enabled: true,
            package_full_name: install.package_full_name.clone(),
            executable: install.executable.clone(),
            created_at: Utc::now().to_rfc3339(),
        }))?;
        let mut engine = InjectorEngine::new(port, browser_id);
        engine.start(payload)?;
        self.engine = Some(engine);
        self.relaunch = None;
        self.status.phase = "active".to_string();
        self.status.message = "背景已应用".to_string();
        self.status.last_error = None;
        self.status.grok_version = Some(install.version.clone());
        Ok(())
    }

    fn write_state(&mut self, state: Option<RuntimeState>) -> Result<(), String> {
        self.state = state;
        match &self.state {
            Some(state) => write_json_transaction(&self.state_path, state),
            None => match fs::remove_file(&self.state_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            },
        }
    }

    fn try_attach_saved(&mut self, install: &GrokInstall) -> bool {
        let Some(state) = &self.state else {
            return false;
        };
        if state.package_full_name != install.package_full_name
            || !state.wco_enabled
            || normalized_path(&state.executable) != normalized_path(&install.executable)
            || read_browser_identity(state.port).ok().as_deref() != Some(state.browser_id.as_str())
            || !session_has_wco(state.port, &state.browser_id)
        {
            return false;
        }
        self.engine = Some(InjectorEngine::new(state.port, state.browser_id.clone()));
        true
    }

    pub fn reconnect_saved(&mut self, payload: ActivePayload) -> Result<bool, String> {
        if self.state.is_none() {
            return Ok(false);
        }
        let install = discover_grok()?;
        if !self.try_attach_saved(&install) {
            self.write_state(None)?;
            self.status = RuntimeStatus::default();
            return Ok(false);
        }
        let result = self
            .engine
            .as_mut()
            .expect("engine set after saved session validation")
            .start(payload);
        match result {
            Ok(()) => {
                self.status.phase = "active".to_string();
                self.status.message = "已自动恢复背景会话".to_string();
                self.status.grok_version = Some(install.version);
                self.status.last_error = None;
                Ok(true)
            }
            Err(error) => {
                self.engine = None;
                self.status.phase = "error".to_string();
                self.status.message = error.clone();
                self.status.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn apply(
        &mut self,
        payload: ActivePayload,
        restart_existing: bool,
    ) -> Result<RuntimeStatus, String> {
        self.watcher_paused = false;
        self.status.phase = "starting".to_string();
        self.status.message = "正在连接 Grok".to_string();
        self.status.last_error = None;
        let result: Result<RuntimeStatus, String> = (|| {
            let install = self.cached_install()?;
            self.drop_disconnected_engine();
            if let Some(engine) = &self.engine {
                match engine.update(payload.clone()) {
                    Ok(()) => {
                        self.status.phase = "active".to_string();
                        self.status.message = "背景已实时应用".to_string();
                        self.status.grok_version = Some(install.version);
                        return Ok(self.status());
                    }
                    Err(error) if is_disconnected_error(&error) => {
                        self.engine = None;
                    }
                    Err(error) => return Err(error),
                }
            }
            if self.try_attach_saved(&install) {
                self.engine
                    .as_mut()
                    .expect("engine set after attach")
                    .start(payload)?;
                self.status.phase = "active".to_string();
                self.status.message = "已重新连接背景会话".to_string();
                self.status.grok_version = Some(install.version);
                return Ok(self.status());
            }
            for port in debug_ports_for(&install)? {
                let Ok(browser_id) = read_browser_identity(port) else {
                    continue;
                };
                if !session_has_wco(port, &browser_id) {
                    continue;
                }
                self.write_state(Some(RuntimeState {
                    schema_version: RUNTIME_SCHEMA_VERSION,
                    port,
                    browser_id: browser_id.clone(),
                    wco_enabled: true,
                    package_full_name: install.package_full_name.clone(),
                    executable: install.executable.clone(),
                    created_at: Utc::now().to_rfc3339(),
                }))?;
                let mut engine = InjectorEngine::new(port, browser_id);
                engine.start(payload.clone())?;
                self.engine = Some(engine);
                self.status.phase = "active".to_string();
                self.status.message = "已重新连接背景会话".to_string();
                self.status.grok_version = Some(install.version);
                return Ok(self.status());
            }
            // 正在等待被结束的旧进程退出：不再重复结束/等待，保持待重启即可。
            if let Some(plan) = self.relaunch.as_mut() {
                if plan.watch.is_none() {
                    let records = snapshot_executable_processes(&install.executable)?;
                    let only_lingering = records
                        .iter()
                        .all(|record| plan.lingering.contains(&record.key));
                    if only_lingering && !grok_fully_exited(&install) {
                        plan.kind = RelaunchKind::Managed;
                        self.show_relaunch_wait(RelaunchKind::Managed);
                        return Ok(self.status());
                    }
                }
            }
            let running = process_ids_for(&install)?;
            if !running.is_empty() && !restart_existing {
                return Err("Grok 需要重启一次以启用背景。".to_string());
            }
            let killed = !running.is_empty();
            let exited = if !killed {
                grok_fully_exited(&install)
            } else {
                stop_verified_grok(&install)?
            };
            if !exited {
                // 已被结束但仍滞留在进程表中，此时启动会被单实例锁或严格快照拦下。
                self.schedule_relaunch(&install, RelaunchKind::Managed, 0, None, Duration::ZERO);
                return Ok(self.status());
            }
            let port = select_port(PREFERRED_DEBUG_PORT)?;
            let launched = match launch_with_transparent_wco(Path::new(&install.executable), port) {
                Ok(launched) => launched,
                // 刚被本插件结束的 Grok 不能就此停在关闭状态：转入待重启继续尝试。
                Err(error) if killed => {
                    self.retry_after_launch_error(
                        &install,
                        RelaunchKind::Managed,
                        0,
                        None,
                        error,
                        false,
                    )?;
                    return Ok(self.status());
                }
                Err(error) => return Err(error),
            };
            match launched {
                WcoLaunch::Ready(browser_id) => {
                    self.attach_launched(&install, port, browser_id, payload)?;
                    Ok(self.status())
                }
                WcoLaunch::ExitedEarly { pid } => {
                    self.schedule_relaunch(
                        &install,
                        RelaunchKind::Managed,
                        1,
                        Some(pid),
                        RELAUNCH_RETRY_INTERVAL,
                    );
                    Ok(self.status())
                }
            }
        })();
        if let Err(error) = &result {
            self.status.phase = if error.contains("需要重启一次") {
                "idle".to_string()
            } else {
                "error".to_string()
            };
            self.status.message = error.clone();
            self.status.last_error = Some(error.clone());
        }
        if result.is_ok() && self.status.phase == "active" {
            self.relaunch = None;
        }
        result
    }

    pub fn pause(&mut self) -> Result<RuntimeStatus, String> {
        self.watcher_paused = true;
        if let Some(engine) = &self.engine {
            engine.pause()?;
        }
        self.status.phase = "paused".to_string();
        self.status.message = MSG_SUSPENDED.to_string();
        self.status.last_error = None;
        Ok(self.status())
    }

    pub fn restore(&mut self) -> Result<RuntimeStatus, String> {
        self.watcher_paused = true;
        self.status.phase = "restoring".to_string();
        self.status.message = "正在恢复官方外观".to_string();
        self.status.last_error = None;
        let result: Result<RuntimeStatus, String> = (|| {
            if let Some(mut engine) = self.engine.take() {
                engine.stop()?;
            }
            match self.cached_install() {
                Ok(install) => {
                    let watching = self
                        .relaunch
                        .as_ref()
                        .is_some_and(|plan| plan.watch.is_some());
                    let mut pending = false;
                    if !process_ids_for(&install)?.is_empty() {
                        if watching {
                            // 刚以官方外观重启过且仍在观察，不再重复结束。
                        } else if stop_verified_grok(&install)? {
                            self.launch_official_watched(&install, RelaunchKind::Official, 0)?;
                        } else {
                            pending = true;
                        }
                    } else if self.relaunch.is_some() && !watching {
                        // 之前被结束的 Grok 尚未重启：改为以官方外观重启。
                        pending = true;
                    } else {
                        crate::fuse_guard::restore_before_official_launch()?;
                    }
                    self.status.grok_version = Some(install.version.clone());
                    if pending {
                        self.schedule_relaunch(
                            &install,
                            RelaunchKind::Official,
                            0,
                            None,
                            Duration::ZERO,
                        );
                    } else if let Some(plan) = self.relaunch.as_mut() {
                        plan.kind = RelaunchKind::Official;
                    }
                }
                Err(_) => {}
            }
            self.write_state(None)?;
            if self
                .relaunch
                .as_ref()
                .is_some_and(|plan| plan.watch.is_none())
            {
                self.show_relaunch_wait(RelaunchKind::Official);
            } else {
                self.status.phase = "idle".to_string();
                self.status.message = "已恢复官方外观".to_string();
            }
            self.status.active_targets = 0;
            Ok(self.status())
        })();
        if let Err(error) = &result {
            self.status.phase = "error".to_string();
            self.status.message = error.clone();
            self.status.last_error = Some(error.clone());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_powershell_literals() {
        assert_eq!(powershell_quote("a'b"), "'a''b'");
    }

    #[test]
    fn selects_an_available_loopback_port() {
        // Let Windows choose outside its reserved port ranges.
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("allocate test port");
        let preferred_port = listener.local_addr().expect("test listener address").port();
        drop(listener);
        let port = select_port(preferred_port).expect("available test port");
        assert!((preferred_port..=preferred_port.saturating_add(100)).contains(&port));
    }

    #[test]
    fn treats_dead_cdp_errors_as_recoverable() {
        assert!(is_disconnected_error("CDP 会话已关闭。"));
        assert!(is_disconnected_error(
            "CDP 浏览器身份已变化，拒绝继续注入。"
        ));
        assert!(!is_disconnected_error("Grok 渲染页执行背景脚本失败：x"));
    }

    #[test]
    fn wait_preserves_failure_until_explicit_action_or_new_process() {
        let dir = std::env::temp_dir().join(format!("grok-managed-status-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut controller = GrokController::load(&dir);
        controller.set_managed_error("探测失败");
        controller.apply_watcher_action_status(WatcherAction::Wait);
        let status = controller.status();
        assert_eq!(status.phase, "error");
        assert_eq!(status.message, "探测失败");
        assert_eq!(status.last_error.as_deref(), Some("探测失败"));

        controller.set_managed_error("探测失败");
        controller.apply_watcher_action_status(WatcherAction::ReportExistingUnmanaged);
        let status = controller.status();
        assert_eq!(status.phase, "idle");
        assert_eq!(status.message, MSG_EXISTING);
        assert!(status.last_error.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires the official Grok desktop installation"]
    fn discovers_installed_grok_and_reads_processes() {
        let install = discover_grok().expect("discover official Grok");
        assert!(normalized_path(&install.executable).ends_with("\\grok bot.exe"));
        process_ids_for(&install).expect("query verified Grok processes");
        for port in debug_ports_for(&install).expect("query verified Grok debug ports") {
            read_browser_identity(port).expect("verify Grok browser identity");
        }
    }
}
