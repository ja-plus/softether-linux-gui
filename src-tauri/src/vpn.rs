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
    /// **正在跑的那个客户端进程**所在目录。
    ///
    /// 和 `vpn_dir`（按设置解析出来的目录）不是一回事：用户改了设置但没重启
    /// 客户端时，两者会不一致。界面靠它判断"显示的是不是真的在跑的那份"。
    pub running_dir: Option<String>,
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
    !running_client_pids().is_empty()
}

/// 正在运行的 SoftEther 客户端进程（`vpnclient execsvc` 的自守护双进程）。
///
/// 直接扫 `/proc` 看 argv，**不用 `pgrep -f`**：后者拿整条命令行做正则，
/// 任何碰巧含有 "vpnclient execsvc" 的命令（编辑器、调试用的 shell、
/// grep 自身）都会被算成客户端。假阳性的后果不只是状态显示错——杀进程
/// 时也会去匹配这些无关命令。
///
/// 判定条件必须是 argv[0] 的 basename 精确等于 vpnclient（或 _x64），
/// 且 argv[1] 就是 execsvc。用户从哪份拷贝起的都算：内置的、Downloads
/// 里的、自己编译的，一律按"当前真在跑的那个"对待。
pub fn running_client_pids() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids: Vec<u32> = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let mut argv = raw.split(|byte| *byte == 0).filter(|part| !part.is_empty());
        let (Some(argv0), Some(argv1)) = (argv.next(), argv.next()) else {
            continue;
        };
        let argv0 = String::from_utf8_lossy(argv0);
        let base = argv0.rsplit('/').next().unwrap_or(&argv0);
        if !matches!(base, "vpnclient" | "vpnclient_x64") {
            continue;
        }
        if String::from_utf8_lossy(argv1) != "execsvc" {
            continue;
        }
        pids.push(pid);
    }
    pids.sort_unstable();
    pids
}

/// 正在跑的那个客户端可执行文件的**真实路径**。
///
/// 停止时必须拿它去跑 `vpnclient stop`，而不是设置里配的那份：
/// 用户很可能自己装了一份在 `~/Downloads`，而应用内置的是另一份。
/// 拿内置那份去 stop 用户那份是停不掉的（客户端靠自己目录里的锁文件
/// 找实例），而 `vpnclient stop` 在这种情况下**仍然以 0 退出**。
pub fn running_client_binary() -> Option<String> {
    let pids = running_client_pids();
    let pid = *pids.first()?;
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv0 = raw.split(|byte| *byte == 0).next()?;
    let path = String::from_utf8_lossy(argv0).to_string();
    (!path.is_empty()).then_some(path)
}

/// 正在跑的客户端进程所在目录（`running_client_binary()` 的父目录）。
pub fn running_client_dir() -> Option<String> {
    let binary = running_client_binary()?;
    let dir = std::path::Path::new(&binary).parent()?;
    Some(settings::path_display(dir))
}

/// 等客户端进程真的消失。等不到就返回 false，调用方据此决定是否补刀。
fn wait_until_stopped(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !probe_service_running() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(150));
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

/// SoftEther 的 Linux 客户端按「虚拟适配器名」建 tap：设备名 = `vpn_` + 适配器名小写。
/// 适配器 `VPN` → `vpn_vpn`，`tet-test` → `vpn_tet-test`。
fn device_name_for_adapter(adapter: &str) -> Option<String> {
    let trimmed = adapter.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("vpn_{}", trimmed.to_lowercase()))
}

/// MAC 归一化：`5E8D018EA0DB`（vpncmd 的输出）与 `5e:8d:01:8e:a0:db`（sysfs）
/// 必须能对上。
fn normalize_mac(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_hexdigit()).map(|c| c.to_ascii_lowercase()).collect()
}

/// 一块候选虚拟网卡的现场快照。
#[derive(Debug, Clone, PartialEq)]
pub struct IfaceCandidate {
    pub name: String,
    pub has_ipv4: bool,
    /// 归一化后的 MAC；读不到就是空串。
    pub mac: String,
}

/// 从候选里挑出"当前该当作隧道网卡"的那一块（纯逻辑，便于测试）。
///
/// **不能按字典序取第一个**：SoftEther 客户端会为**所有**虚拟适配器建 tap，
/// 但只有当前账户那块接了隧道。机器上同时存在 `vpn_tet-test`（闲置）和
/// `vpn_vpn`（账户 tet 在用）时，字典序取第一个正好取到闲置的那块 ——
/// 于是 DHCP 要不到地址、网关推导不出来报 NO_GATEWAY，而真正的隧道网卡
/// 一直没地址、一条路由也没下发。
///
/// 判定顺序：账户适配器名 → 适配器 MAC → 唯一有地址的那块 → 字典序兜底。
pub fn pick_iface(
    candidates: &[IfaceCandidate],
    accounts: &[AccountSummary],
    nics: &[NicInfo],
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let exists = |name: &str| candidates.iter().any(|candidate| candidate.name == name);

    // 1) 已连接的账户优先，其次是正在连的，最后按账户清单顺序
    let mut ordered: Vec<&AccountSummary> = accounts.iter().collect();
    ordered.sort_by_key(|account| state_rank(&account.status));
    for account in &ordered {
        let Some(device) = device_name_for_adapter(&account.nic) else { continue };
        if exists(&device) {
            return Some(device);
        }
    }

    // 2) 名字对不上时按 MAC 认：适配器名里有大小写、空格、非法字符时
    //    系统会改写设备名，只有 MAC 是稳的。
    //
    //    两边都归一化再比：sysfs 是 `5e:96:...`，vpncmd 是 `5E961B...`，
    //    直接字符串比会全部不命中（而且不报错，只是悄悄退回字典序）。
    for account in &ordered {
        let Some(nic) = nics.iter().find(|nic| nic.name == account.nic) else { continue };
        let mac = normalize_mac(&nic.mac);
        if mac.is_empty() {
            continue;
        }
        if let Some(hit) = candidates.iter().find(|candidate| normalize_mac(&candidate.mac) == mac) {
            return Some(hit.name.clone());
        }
    }

    // 3) 只有一块网卡有地址时，它一定就是隧道网卡
    let with_addr: Vec<&IfaceCandidate> = candidates.iter().filter(|c| c.has_ipv4).collect();
    if let [only] = with_addr.as_slice() {
        return Some(only.name.clone());
    }

    // 4) 兜底：至少让界面上有个设备名，别显示"未知"
    candidates.first().map(|candidate| candidate.name.clone())
}

/// 扫一遍 `/sys/class/net` 里的虚拟网卡；有没有 IPv4 用一次 `ip` 调用统一问。
fn iface_candidates() -> Vec<IfaceCandidate> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("vpn_"))
        .collect();
    names.sort();
    let with_addr = settings::interfaces_with_ipv4();
    names
        .into_iter()
        .map(|name| IfaceCandidate {
            has_ipv4: with_addr.contains(&name),
            mac: std::fs::read_to_string(format!("/sys/class/net/{name}/address"))
                .map(|text| normalize_mac(&text))
                .unwrap_or_default(),
            name,
        })
        .collect()
}

/// 当前该当作隧道网卡的设备名。
///
/// 有账户/网卡清单时按「账户 → 适配器 → 设备」认（见 `pick_iface`）；
/// 清单为空（客户端没跑、vpncmd 不可用）时退化成"唯一有地址的那块 / 字典序第一"。
pub fn detect_iface(accounts: &[AccountSummary], nics: &[NicInfo]) -> Option<String> {
    let candidates = iface_candidates();
    if candidates.is_empty() {
        return None;
    }
    pick_iface(&candidates, accounts, nics)
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
    // 网卡要在拿到账户清单之后才认：设备名必须和"哪个账户连着"对上，
    // 机器上可能同时存在多块 tap（见 `pick_iface` 的说明）。
    let iface = detect_iface(&accounts, &nics);

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
        running_dir: if running { running_client_dir() } else { None },
        routes,
        route_warnings,
        link_rx_bytes,
        link_tx_bytes,
    }
}

/// 账户状态的优先级：已连接 > 正在连 > 其它。
///
/// 挑"当前会话"和挑网卡都要用它：正在连的账户虽然还没通，但它比离线账户
/// 更可能是那根正在用的隧道（半连接时网卡已经建好，只是还没地址）。
fn state_rank(status: &str) -> u8 {
    if crate::softether::status_is_connected(status) {
        0
    } else if crate::softether::status_is_connecting(status) {
        1
    } else {
        2
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
        .find(|a| state_rank(&a.status) == 0)
        .map(|a| a.name.clone())
        .or_else(|| accounts.iter().find(|a| state_rank(&a.status) == 1).map(|a| a.name.clone()))
        .or_else(|| accounts.first().map(|a| a.name.clone()))
}

pub fn account_detail(settings: &Settings, name: &str) -> Result<AccountDetail, String> {
    let output = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountGet", name])?;
    if !crate::softether::text_says_success(&output.stdout) {
        return Err(last_error(&output.stdout));
    }
    let mut detail = softether::parse_account_detail(&output.stdout);
    // AccountGet 的输出里没有 UDP 加速项（实测 4.44 只有 20 个字段），
    // 读真实值只能看客户端配置文件；写入侧改的也是这份文件，两边同源。
    if let Some(no_udp) = settings::find_vpn_dir(settings)
        .and_then(|dir| crate::client_dir::account_no_udp_accel(&dir, name))
    {
        detail.no_udp_accel = no_udp;
    }
    Ok(detail)
}

pub fn last_error(output: &str) -> String {
    // vpncmd 拒绝参数时**不会**说"错误"，它说的是
    //   `参数 "/HOST" 已被指定。使用命令 "AccountCreate" 时，不可能指定此参数。`
    // 只找"错误/失败/Error"会漏掉这类最该显示给用户的原因。
    const HINTS: [&str; 8] = [
        "错误",
        "失败",
        "Error",
        "已被指定",
        "不可能指定",
        "不正确",
        "invalid",
        "not found",
    ];
    let line = output
        .lines()
        .rev()
        .find(|line| HINTS.iter().any(|hint| line.contains(hint)))
        .map(|line| line.trim().to_string());
    if let Some(line) = line {
        return line;
    }
    // 实在没有关键词：取最后一条有内容的输出（去掉交互提示行），
    // 比一句"命令没有成功完成"有用得多
    let tail = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("VPN Client>") && !line.starts_with("----"))
        .next_back()
        .map(str::to_string);
    tail.unwrap_or_else(|| {
        crate::i18n::tr(
            "命令没有成功完成",
            "The command did not complete successfully.",
        )
        .to_string()
    })
}

pub fn account_status(settings: &Settings, name: &str) -> Option<SessionSnapshot> {
    account_snapshot(settings, name).filter(|snapshot| snapshot.connected)
}

/// 读一次会话快照，**不管连没连上都返回**（只有 vpncmd 本身失败才是 None）。
///
/// 与 `account_status` 的区别：那个只回答"通了吗"，这个是给连接等待过程解释
/// "卡在哪一步"用的——卡在「重试」和压根没开始连接，处理方式完全不同。
pub fn account_snapshot(settings: &Settings, name: &str) -> Option<SessionSnapshot> {
    let output = settings::vpncmd(settings, settings::VpncmdAuth::Plain, &["AccountStatusGet", name]).ok()?;
    Some(softether::parse_account_status(&output.stdout))
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
        if let Some(code) = output.as_ref().and_then(|o| o.status.code()) {
            if settings::NOT_AUTHORIZED.contains(&code) {
                return Err(settings::explain_not_authorized(""));
            }
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
    // 先走 `vpnclient stop`（让客户端自己收尾，正常路径下会写回配置）。
    let outcome = stop_service_once(settings, wait_seconds);
    // 退出码 0 不算数，只有进程真的消失才算停成功——客户端在这种情况下
    // 也照样打印 "has been stopped"。
    if wait_until_stopped(Duration::from_secs(8)) {
        return Ok(());
    }
    // 授权就没拿到时进程压根没被碰过，补刀只会再弹一个授权框、失败得更慢，
    // 而且最后的"进程仍在运行"会把真正原因盖掉。
    if auth_failed(&outcome) {
        return outcome;
    }
    force_stop_client();
    if wait_until_stopped(Duration::from_secs(6)) {
        return Ok(());
    }
    // 到这里是真的停不掉。把第一手的输出带出去——用户看到的
    // "Unable to create /usr/lib/.../*.VPN-xxxx" 才指向真正原因。
    Err(match outcome {
        Err(message) => message,
        Ok(()) => crate::i18n::tr(
            "客户端没有响应停止指令，进程仍在运行。可以用下面的命令强制结束：sudo pkill -f 'vpnclient execsvc'",
            "The client did not honor the stop request and is still running. Force it with: sudo pkill -f 'vpnclient execsvc'",
        )
        .to_string(),
    })
}

/// 授权环节就已经失败了吗。为真时说明命令压根没执行、进程没被碰过，
/// 后面任何"补刀"都只是多弹一个授权框。
fn auth_failed(result: &Result<(), String>) -> bool {
    match result {
        Ok(()) => false,
        Err(message) => settings::looks_like_not_authorized(message),
    }
}

fn stop_service_once(settings: &Settings, wait_seconds: Option<u64>) -> Result<(), String> {
    // 优先用**正在跑的那份**客户端去 stop：用户自己装的在 ~/Downloads，
    // 应用内置的在 /usr/lib，两者不是同一个二进制。拿错那份去 stop，
    // 客户端会因为找不到实例而什么都不做，而且照样打印 "has been stopped"。
    let binary = running_client_binary().or_else(|| {
        settings::vpnclient_path(settings).map(|p| p.to_string_lossy().to_string())
    });
    if let Some(binary) = binary {
        if settings::privileged_helper().is_some() {
            let output = settings::run_helper_with_timeout(
                &["vpnclient", "stop", &binary],
                wait_seconds,
            )
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
            // 助手把客户端自己的输出原样带回来了，里面才有真正原因
            return Err(stop_still_running(&output));
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
        // pkexec 被取消/没有认证代理时进程压根没被动过，报"仍在运行"是误导
        if let Some(code) = output.as_ref().and_then(|o| o.status.code()) {
            if settings::NOT_AUTHORIZED.contains(&code) {
                return Err(settings::explain_not_authorized(""));
            }
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

/// `vpnclient stop` 之后进程还活着的兜底：直接对 `vpnclient execsvc` 补 SIGTERM。
///
/// 为什么可以直接杀：客户端是**无状态服务**（配置落在 `vpn_client.config`，
/// 隧道由内核侧的会话管理），SIGTERM 打断它不会丢配置，下次 start 照常起来。
/// 而放着不管的代价是界面永远只能报"停止失败"——那是彻底没辙的状态。
///
/// 只有既没装特权助手、也没有可用的免密路径时才走这里。装了助手的情况下
/// 这个收尾由助手脚本自己做（进程在 root 名下，用户态根本杀不掉）。
fn force_stop_client() {
    let pids = running_client_pids();
    if pids.is_empty() {
        return;
    }
    let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
    // 客户端在 root 名下，普通用户杀不掉，必须借一次特权
    let signal = |name: &str| {
        let mut command = Command::new("pkexec");
        command.arg("kill").arg(name).args(&pids);
        let _ = command.status();
    };
    signal("-TERM");
    // 收到 TERM 后客户端会写完配置再退，给它时间
    if wait_until_stopped(Duration::from_secs(5)) {
        return;
    }
    signal("-KILL");
    let _ = wait_until_stopped(Duration::from_secs(3));
}

/// 客户端进程还在跑时给用户的说明。
///
/// 把助手带回来的原始输出接在后面：`vpnclient` 自己在配置写不进去时
/// 只会打印 "Unable to create /usr/lib/.../.VPN-xxxx"，那句话才指向真正原因，
/// 换成一句笼统的"停止失败"，用户只能猜。
fn stop_still_running(output: &str) -> String {
    let detail = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| line.contains("Unable") || line.contains("Error") || line.contains("错误"))
        .collect::<Vec<_>>()
        .join("; ");
    if crate::i18n::is_en() {
        if detail.is_empty() {
            "The client process is still running after stopping.".to_string()
        } else {
            format!("The client process is still running after stopping: {detail}")
        }
    } else if detail.is_empty() {
        "停止客户端后进程仍在运行".to_string()
    } else {
        format!("停止客户端后进程仍在运行：{detail}")
    }
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

    fn account(name: &str, status: &str, nic: &str) -> AccountSummary {
        AccountSummary {
            name: name.into(),
            status: status.into(),
            host: "example.invalid".into(),
            port: Some(12443),
            hub: "VPN".into(),
            nic: nic.into(),
        }
    }

    fn nic(name: &str, mac: &str) -> NicInfo {
        NicInfo { name: name.into(), enabled: true, mac: mac.into() }
    }

    fn candidate(name: &str, has_ipv4: bool, mac: &str) -> IfaceCandidate {
        IfaceCandidate { name: name.into(), has_ipv4, mac: mac.into() }
    }

    /// 真实踩到的坑：机器上躺着两块 tap，账户连着的那块不一定是字典序第一块。
    ///
    /// 账户 `tet` 用适配器 `VPN`（vpn_vpn），另有一块闲置的 `vpn_tet-test`。
    /// 老实现取字典序第一块 → 拿闲置网卡去 DHCP/推导网关 → NO_GATEWAY，
    /// 真正在用的隧道网卡一条路由都没下发。
    #[test]
    fn picks_adapter_of_connected_account_not_alphabetically_first() {
        let candidates = vec![
            candidate("vpn_tet-test", false, "5e:8d:01:8e:a0:db"),
            candidate("vpn_vpn", false, "5e:96:1b:62:14:96"),
        ];
        let accounts = vec![account("tet", "已连接", "VPN")];
        let nics = vec![nic("tet-test", "5E8D018EA0DB"), nic("VPN", "5E961B621496")];
        assert_eq!(pick_iface(&candidates, &accounts, &nics).as_deref(), Some("vpn_vpn"));
    }

    /// 适配器名对不上设备名（系统改写过大小写/非法字符）时按 MAC 认。
    #[test]
    fn falls_back_to_mac_when_device_name_differs() {
        let candidates = vec![
            candidate("vpn_0", false, "5e:8d:01:8e:a0:db"),
            candidate("vpn_1", false, "5e:96:1b:62:14:96"),
        ];
        let accounts = vec![account("tet", "已连接", "VPN")];
        let nics = vec![nic("VPN", "5E961B621496")];
        assert_eq!(pick_iface(&candidates, &accounts, &nics).as_deref(), Some("vpn_1"));
    }

    /// 清单拿不到（客户端没跑/vpncmd 不可用）时，只有一块网卡有地址就认它。
    #[test]
    fn falls_back_to_the_only_iface_with_address() {
        let candidates = vec![
            candidate("vpn_aaa", false, ""),
            candidate("vpn_zzz", true, ""),
        ];
        assert_eq!(pick_iface(&candidates, &[], &[]).as_deref(), Some("vpn_zzz"));
    }

    /// 什么线索都没有时保留老行为：至少给个设备名，不要空着。
    #[test]
    fn keeps_alphabetical_first_as_last_resort() {
        let candidates = vec![
            candidate("vpn_aaa", false, ""),
            candidate("vpn_zzz", false, ""),
        ];
        assert_eq!(pick_iface(&candidates, &[], &[]).as_deref(), Some("vpn_aaa"));
        assert_eq!(pick_iface(&[], &[], &[]), None, "一块网卡都没有时必须返回 None");
    }

    /// 适配器名 → 设备名的规则（vpncmd 里是大写 `VPN`，设备是小写 `vpn_vpn`）。
    #[test]
    fn device_name_lowercases_adapter() {
        assert_eq!(device_name_for_adapter("VPN").as_deref(), Some("vpn_vpn"));
        assert_eq!(device_name_for_adapter("tet-test").as_deref(), Some("vpn_tet-test"));
        assert_eq!(device_name_for_adapter("  ").as_deref(), None);
    }
}
