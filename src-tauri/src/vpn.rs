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
    /// 虚拟网卡的累计收发字节（直接读 sysfs）。
    ///
    /// 界面上的实时速率必须用它，不能用 `snapshot.rx_bytes`——后者来自
    /// `AccountStatusGet`，每读一次要冷启动一个 vpncmd 进程（实测 250ms、
    /// 启动瞬间吃满一个核），2 秒一轮的速率刷新根本付不起这个代价。
    /// 会话层的累计量仍走 vpncmd，低频更新即可。
    pub link_rx_bytes: u64,
    pub link_tx_bytes: u64,
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

/// `AccountList` + `NicList` 的结果缓存。
///
/// 这两份清单只在用户增删账号、改配置时变化，却原来跟着 2 秒的轮询每轮各
/// 冷启动一次 vpncmd（实测 250 / 275 ms，启动瞬间吃满一个核）。缓存之后
/// 稳态轮询不再为此付出任何进程开销——这正是"界面每两秒顿一下"的来源。
static INVENTORY: Mutex<Option<InventoryCache>> = Mutex::new(None);
const INVENTORY_TTL: Duration = Duration::from_secs(30);

struct InventoryCache {
    at: Instant,
    accounts: Vec<AccountSummary>,
    nics: Vec<NicInfo>,
}

/// `AccountStatusGet` 的结果缓存。
///
/// 连接状态的变化远慢于 2 秒，而且所有会改变它的动作（连接、断开、启停服务）
/// 都走 `read_overview_fresh`，缓存会被一并重建，不存在读到旧状态的窗口。
static STATUS: Mutex<Option<StatusCache>> = Mutex::new(None);
/// 5 秒：`sysfs` 的流量是秒级实时的，这个 TTL 只决定"连接状态"多久复核一次。
/// 意外断线最多晚 5 秒发现，换来稳态下 vpncmd 的占空比从 38% 降到 5%。
const STATUS_TTL: Duration = Duration::from_secs(5);

struct StatusCache {
    at: Instant,
    name: String,
    /// `None` 表示这个账号当前未连接；未连接也要缓存，否则每轮都会白 fork 一次
    snapshot: Option<SessionSnapshot>,
}

/// 把两层缓存清空。操作路径（连接/断开/启停）读状态前必须调用。
pub fn clear_overview_cache() {
    if let Ok(mut cache) = INVENTORY.lock() {
        *cache = None;
    }
    if let Ok(mut cache) = STATUS.lock() {
        *cache = None;
    }
}

fn cached_inventory(now: Instant) -> Option<(Vec<AccountSummary>, Vec<NicInfo>)> {
    let cache = INVENTORY.lock().ok()?;
    let entry = cache.as_ref()?;
    (now.duration_since(entry.at) < INVENTORY_TTL)
        .then(|| (entry.accounts.clone(), entry.nics.clone()))
}

fn store_inventory(at: Instant, accounts: Vec<AccountSummary>, nics: Vec<NicInfo>) {
    if let Ok(mut cache) = INVENTORY.lock() {
        *cache = Some(InventoryCache { at, accounts, nics });
    }
}

/// 读账户与网卡清单。任一路读失败都不写缓存——客户端可能正在重启，
/// 把一份空清单钉住 30 秒会让界面以为账号都丢了。
fn read_inventory(settings: &Settings, now: Instant) -> (Vec<AccountSummary>, Vec<NicInfo>) {
    if let Some(hit) = cached_inventory(now) {
        return hit;
    }
    let mut accounts = Vec::new();
    let mut nics = Vec::new();
    let mut complete = true;

    match settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountList"]) {
        Ok(output) if crate::softether::text_says_success(&output.stdout) => {
            accounts = softether::parse_account_list(&output.stdout);
        }
        _ => complete = false,
    }
    match settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["NicList"]) {
        Ok(output) if crate::softether::text_says_success(&output.stdout) => {
            nics = softether::parse_nic_list(&output.stdout);
        }
        _ => complete = false,
    }
    if complete {
        store_inventory(now, accounts.clone(), nics.clone());
    }
    (accounts, nics)
}

fn cached_status(now: Instant, name: &str) -> Option<Option<SessionSnapshot>> {
    let cache = STATUS.lock().ok()?;
    let entry = cache.as_ref()?;
    if entry.name == name && now.duration_since(entry.at) < STATUS_TTL {
        return Some(entry.snapshot.clone());
    }
    None
}

fn read_status(settings: &Settings, name: &str, now: Instant) -> Option<SessionSnapshot> {
    if let Some(hit) = cached_status(now, name) {
        return hit;
    }
    let output = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountStatusGet", name]).ok()?;
    let parsed = softether::parse_account_status(&output.stdout);
    let snapshot = parsed.connected.then_some(parsed);
    if let Ok(mut cache) = STATUS.lock() {
        *cache = Some(StatusCache { at: now, name: name.to_string(), snapshot: snapshot.clone() });
    }
    snapshot
}

/// 网卡累计收发字节，读 sysfs 是一次文件读，不 fork 任何进程。
fn link_counters(iface: &str) -> Option<(u64, u64)> {
    let read = |field: &str| -> Option<u64> {
        std::fs::read_to_string(format!("/sys/class/net/{iface}/statistics/{field}"))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((read("rx_bytes")?, read("tx_bytes")?))
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

/// 读一份状态，走缓存。高频轮询（前端 2 秒一轮）专用。
pub fn read_overview(settings: &Settings) -> Overview {
    build_overview(settings, false)
}

/// 读一份状态，忽略缓存。连接/断开/启停这类操作之后必须用这个，
/// 否则会把动作之前的状态当成结果回填给界面。
pub fn read_overview_fresh(settings: &Settings) -> Overview {
    build_overview(settings, true)
}

fn build_overview(settings: &Settings, fresh: bool) -> Overview {
    if fresh {
        clear_overview_cache();
    }
    let now = Instant::now();
    // 轮询路径用缓存版探测：每 2 秒 fork 一次 pgrep 没必要
    let running = service_running_cached();
    let iface = detect_iface();
    let vpncmd_ready = settings::vpncmd_path(settings).is_some();
    let (accounts, nics, snapshot) = if vpncmd_ready && running {
        let (accounts, nics) = read_inventory(settings, now);
        let snapshot = active_account(settings, &accounts)
            .and_then(|name| read_status(settings, &name, now));
        (accounts, nics, snapshot)
    } else {
        // 客户端没跑：清缓存，下次启动绝不能拿停机前的清单
        clear_overview_cache();
        (Vec::new(), Vec::new(), None)
    };

    let routes = iface.as_deref().map(settings::routes_on).unwrap_or_default();
    let (link_rx_bytes, link_tx_bytes) = iface.as_deref().and_then(link_counters).unwrap_or((0, 0));
    let mut route_warnings = Vec::new();
    if snapshot.is_none() && !routes.is_empty() {
        let iface_name = iface.clone().unwrap_or_else(|| {
            crate::i18n::tr("虚拟网卡", "virtual adapter").to_string()
        });
        route_warnings.push(if crate::i18n::is_en() {
            format!(
                "{} still has {} subnet route(s) while the tunnel is down; intranet traffic may be black-holed",
                iface_name,
                routes.len()
            )
        } else {
            format!(
                "{} 上仍有 {} 条网段路由，但隧道未连接，内网可能被黑洞",
                iface_name,
                routes.len()
            )
        });
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
        link_rx_bytes,
        link_tx_bytes,
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
    line.unwrap_or_else(|| {
        crate::i18n::tr(
            "命令没有成功完成",
            "The command did not complete successfully.",
        )
        .to_string()
    })
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
fn explain_failure(action_zh: &str, action_en: &str, status: Option<i32>) -> String {
    if crate::i18n::is_en() {
        match status {
            Some(126) | Some(127) => format!("Authorization cancelled; client was not {action_en}"),
            _ => format!("Failed to {action_en} the client"),
        }
    } else {
        match status {
            Some(126) | Some(127) => format!("已取消授权，未{action_zh}客户端"),
            _ => format!("{action_zh}客户端失败"),
        }
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
            settings::run_helper(&["vpnclient", "start", &binary]).map_err(|e| {
                if crate::i18n::is_en() {
                    format!("Failed to start the VPN service: {e}")
                } else {
                    format!("启动 VPN 服务失败: {e}")
                }
            })?;
            if service_running() {
                return Ok(true);
            }
            return Err(crate::i18n::tr(
                "启动客户端后仍未见进程，请检查 vpnclient 日志",
                "The client process still is not running; check the vpnclient log.",
            )
            .into());
        }
        let output = run_root(&binary, "start", None).map_err(|e| {
            if crate::i18n::is_en() {
                format!("Failed to start the VPN service: {e}")
            } else {
                format!("启动 VPN 服务失败: {e}")
            }
        })?;
        if service_running() {
            return Ok(true);
        }
        return Err(explain_failure("启动", "start", output.and_then(|o| o.status.code())));
    }
    if which("systemctl").is_some() {
        let _ = Command::new("pkexec").args(["systemctl", "start", "vpnclient"]).status();
        if service_running() {
            return Ok(true);
        }
    }
    Err(crate::i18n::tr(
        "找不到 vpnclient，且系统服务未能启动。请在设置里指定 SoftEther VPN Client 目录",
        "Cannot find vpnclient and the system service did not start. Set the SoftEther VPN Client directory in Settings.",
    )
    .into())
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
                .map_err(|e| {
                    if crate::i18n::is_en() {
                        format!("Failed to stop the VPN service: {e}")
                    } else {
                        format!("停止 VPN 服务失败: {e}")
                    }
                })?;
            if !service_running() {
                return Ok(());
            }
            return Err(crate::i18n::tr(
                "停止客户端后进程仍在运行",
                "The client process is still running after stopping.",
            )
            .into());
        }
        let output = run_root(&binary, "stop", wait_seconds).map_err(|e| {
            if crate::i18n::is_en() {
                format!("Failed to stop the VPN service: {e}")
            } else {
                format!("停止 VPN 服务失败: {e}")
            }
        })?;
        if !service_running() {
            return Ok(());
        }
        if output.is_none() {
            return Err(crate::i18n::tr(
                "等待授权超时，客户端未停止",
                "Timed out waiting for authorization; the client was not stopped.",
            )
            .into());
        }
        return Err(explain_failure("停止", "stop", output.and_then(|o| o.status.code())));
    }
    if which("systemctl").is_some() {
        let _ = Command::new("pkexec").args(["systemctl", "stop", "vpnclient"]).status();
        if !service_running() {
            return Ok(());
        }
    }
    Err(crate::i18n::tr(
        "停止 VPN 服务失败，客户端进程仍在运行",
        "Failed to stop the VPN service; the client process is still running.",
    )
    .into())
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

    /// 速率数据源必须能直接读，不能退化到"读不到就显示 0"。
    #[test]
    fn link_counters_reads_sysfs() {
        let (rx, tx) = link_counters("lo").expect("lo 的计数器必须可读");
        // lo 上跑过任何测试流量都不该让这两项变成 0 以外的怪值；这里只验路径拼接正确
        assert!(rx > 0 || tx > 0, "lo 计数器应当有累计值（本机任何网络活动都会计入）");
        assert!(link_counters("softether-no-such-iface").is_none(), "不存在的网卡必须返回 None");
    }

    /// 缓存命中时必须完全不碰 vpncmd（否则等于没优化）。
    #[test]
    fn overview_cache_short_circuits_refresh() {
        let settings = Settings::default();
        if !service_running_cached() {
            // 客户端没跑：这条路径本来就不读 vpncmd，跳过环境相关的断言
            return;
        }
        clear_overview_cache();
        store_inventory(
            Instant::now(),
            vec![AccountSummary {
                name: "cached-account".into(),
                status: "已连接".into(),
                host: "example.invalid".into(),
                port: Some(443),
                hub: "VPN".into(),
                nic: "VPN".into(),
            }],
            Vec::new(),
        );
        let (accounts, _) = read_inventory(&settings, Instant::now());
        assert_eq!(accounts.len(), 1, "命中缓存必须直接返回缓存内容，不再启动 vpncmd");
        clear_overview_cache();
    }
}
