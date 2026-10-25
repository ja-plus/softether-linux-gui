use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::settings::{self, which, Settings};
use crate::softether::{self, AccountDetail, AccountSummary, NicInfo, SessionSnapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub service_running: bool,
    pub iface: Option<String>,
    pub iface_ipv4: Option<String>,
    pub iface_mtu: Option<u16>,
    pub accounts: Vec<AccountSummary>,
    pub nics: Vec<NicInfo>,
    pub snapshot: Option<SessionSnapshot>,
    pub vpncmd_ready: bool,
    pub vpn_dir: Option<String>,
    pub routes: Vec<String>,
    pub route_warnings: Vec<String>,
}

/// `vpnclient` 进程探测结果的短缓存。
///
/// `read_overview` 每 2 秒一轮，而一次探测要 fork 一个 pgrep；缓存只活 2 秒。
/// 启停服务之后必须调 `invalidate_service_cache()`，否则紧接着的状态读取
/// 会拿到动作之前的旧答案。
static SERVICE_CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
const SERVICE_CACHE_TTL: Duration = Duration::from_secs(2);

fn probe_service_running() -> bool {
    if std::path::Path::new("/run/softether-vpnclient.pid").is_file() {
        return true;
    }
    match Command::new("pgrep").args(["-f", "(^|/)vpnclient execsvc"]).stdout(Stdio::null()).status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

/// 实时探测客户端进程，不吃缓存。启停服务前后的判定必须用这个。
pub fn service_running() -> bool {
    probe_service_running()
}

/// 高频轮询专用：结果缓存 2 秒，挡住反复 fork pgrep。
pub fn service_running_cached() -> bool {
    let now = Instant::now();
    if let Ok(cache) = SERVICE_CACHE.lock() {
        if let Some((at, value)) = *cache {
            if now.duration_since(at) < SERVICE_CACHE_TTL {
                return value;
            }
        }
    }
    let value = probe_service_running();
    if let Ok(mut cache) = SERVICE_CACHE.lock() {
        *cache = Some((now, value));
    }
    value
}

/// 客户端状态刚被本应用改变过：作废缓存，下一次读取立刻重新探测。
pub fn invalidate_service_cache() {
    if let Ok(mut cache) = SERVICE_CACHE.lock() {
        *cache = None;
    }
}

pub fn detect_iface() -> Option<String> {
    let mut entries: Vec<String> = std::fs::read_dir("/sys/class/net")
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("vpn_"))
        .collect();
    entries.sort();
    entries.into_iter().next()
}

pub fn read_overview(settings: &Settings) -> Overview {
    // 轮询路径用缓存版探测：每 2 秒 fork 一次 pgrep 没必要
    let running = service_running_cached();
    let iface = detect_iface();
    let mut accounts = Vec::new();
    let mut nics = Vec::new();
    let mut snapshot = None;
    let vpncmd_ready = settings::vpncmd_path(settings).is_some();

    if vpncmd_ready && running {
        if let Ok(output) = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountList"]) {
            if crate::softether::text_says_success(&output.stdout) {
                accounts = softether::parse_account_list(&output.stdout);
            }
        }
        if let Ok(output) = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["NicList"]) {
            if crate::softether::text_says_success(&output.stdout) {
                nics = softether::parse_nic_list(&output.stdout);
            }
        }
        let target = active_account(settings, &accounts);
        if let Some(name) = target {
            if let Ok(output) = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountStatusGet", &name]) {
                let parsed = softether::parse_account_status(&output.stdout);
                if parsed.connected {
                    snapshot = Some(parsed);
                }
            }
        }
    }

    let routes = iface.as_deref().map(settings::routes_on).unwrap_or_default();
    let mut route_warnings = Vec::new();
    if snapshot.is_none() && !routes.is_empty() {
        route_warnings.push(format!(
            "{} 上仍有 {} 条网段路由，但隧道未连接，内网可能被黑洞",
            iface.clone().unwrap_or_else(|| "虚拟网卡".into()),
            routes.len()
        ));
    }

    Overview {
        service_running: running,
        iface_ipv4: iface.as_deref().and_then(settings::interface_ipv4),
        iface_mtu: iface.as_deref().and_then(settings::interface_mtu),
        iface,
        accounts,
        nics,
        snapshot,
        vpncmd_ready,
        vpn_dir: settings::find_vpn_dir(settings).map(|p| settings::path_display(&p)),
        routes,
        route_warnings,
    }
}

pub fn active_account(settings: &Settings, accounts: &[AccountSummary]) -> Option<String> {
    if let Some(name) = &settings.active_account {
        if accounts.iter().any(|a| &a.name == name) {
            return Some(name.clone());
        }
    }
    accounts
        .iter()
        .find(|a| crate::softether::status_is_connected(&a.status))
        .map(|a| a.name.clone())
        .or_else(|| accounts.first().map(|a| a.name.clone()))
}

pub fn account_detail(settings: &Settings, name: &str) -> Result<AccountDetail, String> {
    let output = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountGet", name])?;
    if !crate::softether::text_says_success(&output.stdout) {
        return Err(last_error(&output.stdout));
    }
    Ok(softether::parse_account_detail(&output.stdout))
}

pub fn last_error(output: &str) -> String {
    let line = output
        .lines()
        .rev()
        .find(|line| line.contains("错误") || line.contains("失败") || line.contains("Error"))
        .map(|line| line.trim().to_string());
    line.unwrap_or_else(|| "命令没有成功完成".to_string())
}

pub fn account_status(settings: &Settings, name: &str) -> Option<SessionSnapshot> {
    let output = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountStatusGet", name]).ok()?;
    let parsed = softether::parse_account_status(&output.stdout);
    parsed.connected.then_some(parsed)
}

/// 以 root 执行一条无参命令。
///
/// 优先 `sudo -n`：装了 sudoers 白名单时完全静默、无弹窗；
/// 未装时 `sudo -n` 会立刻失败（不挂起），自动退回 pkexec 弹一次密码框。
///
/// `wait_seconds` 为 `None` 表示一直等授权；传入秒数则在超时后返回 `Ok(None)`，
/// 由调用方决定超时怎么处理。
pub(crate) fn run_root(
    binary: &str,
    action: &str,
    wait_seconds: Option<u64>,
) -> std::io::Result<Option<std::process::Output>> {
    if let Some(sudo) = which("sudo") {
        if let Ok(output) = Command::new(&sudo)
            .args(["-n", binary, action])
            .stdin(Stdio::null())
            .output()
        {
            if output.status.success() {
                return Ok(Some(output));
            }
        }
    }
    let mut child = Command::new("pkexec")
        .arg(binary)
        .arg(action)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(seconds) = wait_seconds {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        loop {
            if let Some(_status) = child.try_wait()? {
                return child.wait_with_output().map(Some);
            }
            if std::time::Instant::now() >= deadline {
                // 超时就不再等授权，把弹窗收掉，避免留下孤儿对话框
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    child.wait_with_output().map(Some)
}

/// 把 pkexec / sudo 的退出码翻译成人能看懂的失败原因。
///
/// 126 = 用户取消了授权框（或没有授权代理），127 = 找不到命令。
fn explain_failure(action: &str, status: Option<i32>) -> String {
    match status {
        Some(126) | Some(127) => format!("已取消授权，未{action}客户端"),
        _ => format!("{action}客户端失败"),
    }
}

/// 启动 SoftEther 客户端服务。
///
/// 返回 `Ok(true)` 表示**本次调用真的把客户端拉起来了**，`Ok(false)` 表示客户端
/// 本来就在跑、什么都没做。调用方必须凭这个返回值决定是否写托管标记——客户端
/// 是自守护的共享进程，用户可能自己先开着；把它误标成"本应用拉起"会让应用退出
/// 时关掉用户自己的隧道。
pub fn start_service(settings: &Settings) -> Result<bool, String> {
    let result = start_service_inner(settings);
    // 客户端状态刚变过，作废探测缓存，免得紧接着的状态读取拿到旧答案
    invalidate_service_cache();
    result
}

fn start_service_inner(settings: &Settings) -> Result<bool, String> {
    if service_running() {
        return Ok(false);
    }
    if let Some(binary) = settings::vpnclient_path(settings) {
        let binary = binary.to_string_lossy().to_string();
        // 装了 polkit 助手就静默启动；否则退回 pkexec / sudo 弹框
        if settings::privileged_helper().is_some() {
            settings::run_helper(&["vpnclient", "start", &binary])
                .map_err(|e| format!("启动 VPN 服务失败: {e}"))?;
            if service_running() {
                return Ok(true);
            }
            return Err("启动客户端后仍未见进程，请检查 vpnclient 日志".into());
        }
        let output = run_root(&binary, "start", None).map_err(|e| format!("启动 VPN 服务失败: {e}"))?;
        if service_running() {
            return Ok(true);
        }
        return Err(explain_failure("启动", output.and_then(|o| o.status.code())));
    }
    if which("systemctl").is_some() {
        let _ = Command::new("pkexec").args(["systemctl", "start", "vpnclient"]).status();
        if service_running() {
            return Ok(true);
        }
    }
    Err("找不到 vpnclient，且系统服务未能启动。请在设置里指定 SoftEther VPN Client 目录".into())
}

/// 停止 SoftEther 客户端服务。
///
/// 先走 `vpnclient stop`（正常路径，会让客户端自行收尾），失败再退回 systemctl。
/// 客户端是自守护的双进程，`stop` 之后必须复核，不能只看退出码。
///
/// `wait_seconds` 控制等待授权的最长时间：退出应用时传一个较小的值，
/// 免得没装 sudoers 白名单时被密码框卡住，退出本身才是用户意图。
pub fn stop_service(settings: &Settings, wait_seconds: Option<u64>) -> Result<(), String> {
    let result = stop_service_inner(settings, wait_seconds);
    // 同 start_service：状态刚变过，作废探测缓存
    invalidate_service_cache();
    result
}

fn stop_service_inner(settings: &Settings, wait_seconds: Option<u64>) -> Result<(), String> {
    if !service_running() {
        return Ok(());
    }
    if let Some(binary) = settings::vpnclient_path(settings) {
        let binary = binary.to_string_lossy().to_string();
        if settings::privileged_helper().is_some() {
            settings::run_helper_with_timeout(&["vpnclient", "stop", &binary], wait_seconds)
                .map_err(|e| format!("停止 VPN 服务失败: {e}"))?;
            if !service_running() {
                return Ok(());
            }
            return Err("停止客户端后进程仍在运行".into());
        }
        let output = run_root(&binary, "stop", wait_seconds)
            .map_err(|e| format!("停止 VPN 服务失败: {e}"))?;
        if !service_running() {
            return Ok(());
        }
        if output.is_none() {
            return Err("等待授权超时，客户端未停止".into());
        }
        return Err(explain_failure("停止", output.and_then(|o| o.status.code())));
    }
    if which("systemctl").is_some() {
        let _ = Command::new("pkexec").args(["systemctl", "stop", "vpnclient"]).status();
        if !service_running() {
            return Ok(());
        }
    }
    Err("停止 VPN 服务失败，客户端进程仍在运行".into())
}

/// 客户端进程是否由本应用拉起（用于决定退出时该不该停它）
pub fn managed_by_app() -> bool {
    std::path::Path::new(&crate::settings::managed_flag_path()).is_file()
}

pub fn mark_managed() {
    let path = crate::settings::managed_flag_path();
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{}\n", std::process::id()));
}

pub fn clear_managed() {
    let _ = std::fs::remove_file(crate::settings::managed_flag_path());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidate_clears_service_cache() {
        let _ = service_running_cached();
        assert!(SERVICE_CACHE.lock().unwrap().is_some(), "缓存版探测应当写入缓存");
        invalidate_service_cache();
        assert!(SERVICE_CACHE.lock().unwrap().is_none(), "失效后下一次读取必须重新探测");
    }
}
