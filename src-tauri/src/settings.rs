use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 路由表只由这里拼装；前端永远提交完整、已校验的网段列表。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteConfig {
    #[serde(default = "default_subnets")]
    pub subnets: Vec<String>,
    #[serde(default)]
    pub auto_connect: bool,
/// 断开时是否连同虚拟网卡上的 IPv4 地址一起清掉。
///
/// 地址是连接时用 `dhclient -1` 要来的，而 `-1` 表示"要完就退出"，没有常驻进程
/// 会在链路失效时自动释放租约，所以这一步得我们自己收尾——默认开启。
#[serde(default = "default_true")]
pub dhcp: bool,
    #[serde(default = "default_mtu")]
    pub mtu: u16,
}

fn default_subnets() -> Vec<String> {
    vec!["10.0.0.0/8".into(), "172.16.0.0/12".into(), "192.168.0.0/16".into()]
}

fn default_mtu() -> u16 {
    1400
}

impl Default for RouteConfig {
    fn default() -> Self {
        Self { subnets: default_subnets(), auto_connect: false, dhcp: true, mtu: default_mtu() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub vpn_dir: Option<String>,
    #[serde(default)]
    pub vpncmd_path: Option<String>,
    #[serde(default = "default_account")]
    pub active_account: Option<String>,
    #[serde(default = "default_true")]
    pub notifications: bool,
    #[serde(default = "default_true")]
    pub start_minimized: bool,
    #[serde(default)]
    pub diag_target: String,
    #[serde(default)]
    pub routes: RouteConfig,
    /// 应用启动时自动启动 SoftEther 客户端服务
    #[serde(default)]
    pub start_client_on_launch: bool,
    /// 应用退出时停止客户端服务（仅当客户端由本应用拉起）
    #[serde(default)]
    pub stop_client_on_exit: bool,
    /// 界面风格："win11" 或 "deepin"。配置文件里被手改成别的值时，
    /// 保存时归一化；渲染侧同样只认这两种，未知值一律按 win11 处理。
    #[serde(default = "default_ui_style")]
    pub ui_style: String,
}

fn default_account() -> Option<String> {
    None
}

fn default_true() -> bool {
    true
}

fn default_ui_style() -> String {
    "win11".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vpn_dir: None,
            vpncmd_path: None,
            active_account: default_account(),
            notifications: true,
            start_minimized: true,
            diag_target: "10.10.4.160".into(),
            routes: RouteConfig::default(),
            start_client_on_launch: false,
            stop_client_on_exit: false,
            ui_style: default_ui_style(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs_home().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("softether-tray")
}

/// 标记文件：存在即表示当前运行的 vpnclient 是本应用拉起的
pub fn managed_flag_path() -> String {
    config_dir().join("client-managed.flag").to_string_lossy().to_string()
}

pub fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

impl Settings {
    /// 配置文件缺失或被手改坏时只回退到默认值，不覆盖用户原文件。
    pub fn load() -> Self {
        let path = settings_path();
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let mut loaded: Self = serde_json::from_str(&text).unwrap_or_default();
        loaded.normalize_ui_style();
        loaded
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = config_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录 {}: {e}", dir.display()))?;
        let path = settings_path();
        let tmp = path.with_extension("json.tmp");
        // 写回前顺手治愈手改坏的风格取值，避免垃圾值一直留在文件里
        let mut normalized = self.clone();
        normalized.normalize_ui_style();
        let text = serde_json::to_string_pretty(&normalized).map_err(|e| e.to_string())?;
        {
            let mut file = fs::File::create(&tmp).map_err(|e| format!("无法写入配置: {e}"))?;
            file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        }
        fs::rename(&tmp, &path).map_err(|e| format!("无法替换配置: {e}"))
    }

    /// 把界面风格收敛到已知取值（win11 / deepin），手改坏的按 win11 处理。
    fn normalize_ui_style(&mut self) {
        if self.ui_style != "deepin" {
            self.ui_style = default_ui_style();
        }
    }
}

pub fn find_vpn_dir(settings: &Settings) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = &settings.vpn_dir {
        candidates.push(PathBuf::from(dir));
    }
    major_candidates(&mut candidates);
    candidates.into_iter().find(|dir| dir.join("vpncmd").is_file())
}

/// deb 包内置的客户端目录。装了本应用就一定有，优先级最高。
pub const BUNDLED_CLIENT_DIR: &str = "/usr/lib/softether-tray/client";

fn major_candidates(into: &mut Vec<PathBuf>) {
    // 内置客户端排最前：它随包安装，版本确定、语言锁定为 cn，
    // 是唯一"一定存在且行为可预期"的候选。
    into.push(PathBuf::from(BUNDLED_CLIENT_DIR));
    if let Some(home) = dirs_home() {
        into.push(home.join("Downloads/vpnclient"));
        into.push(home.join("opt/softether-vpnclient"));
    }
    into.push(PathBuf::from("/usr/local/vpnclient"));
    into.push(PathBuf::from("/opt/softether-vpnclient"));
}

pub fn vpncmd_path(settings: &Settings) -> Option<PathBuf> {
    if let Some(explicit) = &settings.vpncmd_path {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(dir) = find_vpn_dir(settings) {
        return Some(dir.join("vpncmd"));
    }
    which("vpncmd")
}

pub fn vpnclient_path(settings: &Settings) -> Option<PathBuf> {
    find_vpn_dir(settings).map(|dir| dir.join("vpnclient")).filter(|p| p.is_file())
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
}

/// 在几个固定目录里找可执行文件。
///
/// 桌面会话启动的 Tauri 应用，PATH 往往不含 `/sbin` 与 `/usr/sbin`（Debian 系默认如此），
/// 只靠 `which` 会把装机时就在的 `dhclient` 判成"没装"。这里额外扫固定系统目录。
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(found) = which(name) {
        return Some(found);
    }
    ["/sbin", "/usr/sbin", "/usr/local/sbin", "/bin", "/usr/bin", "/usr/local/bin"]
        .iter()
        .map(|dir| PathBuf::from(dir).join(name))
        .find(|candidate| candidate.is_file())
}

pub enum VpncmdAuth {
    Plain,
    Elevated,
}

#[derive(Debug)]
pub struct VpncmdOutput {
    pub stdout: String,
    pub code: i32,
}

pub fn vpncmd(settings: &Settings, auth: VpncmdAuth, args: &[&str]) -> Result<VpncmdOutput, String> {
    let binary = vpncmd_path(settings).ok_or_else(|| "找不到 vpncmd，请在设置里指定 SoftEther VPN Client 目录".to_string())?;
    let mut command = match auth {
        VpncmdAuth::Plain => Command::new(&binary),
        // pkexec 走图形密码框，避免 GUI 里出现没有 stdin 的 sudo 卡死。
        VpncmdAuth::Elevated => {
            let pkexec = which("pkexec").ok_or_else(|| "系统缺少 pkexec，无法申请管理员权限".to_string())?;
            let mut cmd = Command::new(pkexec);
            cmd.arg(&binary);
            cmd
        }
    };
    command
        .arg("localhost")
        .arg("/CLIENT")
        .arg("/CMD")
        .args(args)
        // LC_ALL 决定多字节字符能否正确显示；LANG 一起给，某些发行版只认 LANG。
        // 注意：**输出语言的开关是客户端目录下的 lang.config，不是这两个变量**。
        // 解析层已做成中/英/日三语兼容，这里只是保证中文不乱码。
        .env("LC_ALL", "zh_CN.UTF-8")
        .env("LANG", "zh_CN.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.output().map_err(|e| format!("执行 vpncmd 失败: {e}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        text.push('\n');
        text.push_str(&stderr);
    }
    Ok(VpncmdOutput { stdout: text, code: output.status.code().unwrap_or(-1) })
}

pub fn validate_subnet(value: &str) -> Result<(), String> {
    let (addr, prefix) = value.split_once('/').ok_or_else(|| format!("网段 {value} 缺少前缀长度"))?;
    let prefix: u8 = prefix.parse().map_err(|_| format!("网段 {value} 前缀不是数字"))?;
    if prefix > 32 {
        return Err(format!("网段 {value} 前缀超过 32"));
    }
    let mut octets = 0;
    for part in addr.split('.') {
        part.parse::<u8>().map_err(|_| format!("网段 {value} 不是合法 IPv4"))?;
        octets += 1;
    }
    if octets != 4 {
        return Err(format!("网段 {value} 不是合法 IPv4"));
    }
    Ok(())
}

pub fn validate_target(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("诊断目标不能为空".into());
    }
    if value.len() > 253 || value.starts_with('-') {
        return Err("诊断目标不合法".into());
    }
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':')) {
        return Err("诊断目标只能包含字母、数字、点、连字符和冒号".into());
    }
    Ok(())
}

pub fn validate_mtu(mtu: u16) -> Result<(), String> {
    if (576..=1500).contains(&mtu) {
        Ok(())
    } else {
        Err("MTU 必须介于 576 和 1500 之间".into())
    }
}

pub fn run_privileged(script: &str) -> Result<String, String> {
    let pkexec = which("pkexec").ok_or_else(|| "系统缺少 pkexec".to_string())?;
    let output = Command::new(pkexec)
        .arg("sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("执行提权命令失败: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!("提权操作失败（可能是取消了密码框）: {}{}", stdout.trim(), stderr.trim()))
    }
}

/// 特权网络代理脚本的安装路径。
///
/// 装了 polkit 授权（`Tools/install-polkit.sh`）之后存在。它把网卡操作收窄成
/// 一个参数受限的固定程序，polkit 规则只放行它一个，于是连接时不再弹密码框。
pub const PRIVILEGED_HELPER: &str = "/usr/libexec/softether-tray/softether-tray-net";

/// 检查 helper 是否可用且至少能被 root 之外的进程读到（存在即视为已安装）。
pub fn privileged_helper() -> Option<PathBuf> {
    let path = PathBuf::from(PRIVILEGED_HELPER);
    path.is_file().then_some(path)
}

/// 通过 helper 执行一次特权操作（走 pkexec，命中 polkit 规则时静默通过）。
///
/// 参数全部走独立 argv，不拼 shell，helper 内部自己再校验一遍——
/// 两边各校验一次不是重复，是防止其中一处被改动时直接裸奔。
pub fn run_helper(args: &[&str]) -> Result<String, String> {
    run_helper_with_timeout(args, None)
}

/// 同上，`wait_seconds` 为 `Some(n)` 时最多等 n 秒授权，超时返回 `Err(超时)`。
///
/// 退出应用时要用短的超时：那会儿的用户意图是"关掉窗口"，不该被密码框卡住。
pub fn run_helper_with_timeout(args: &[&str], wait_seconds: Option<u64>) -> Result<String, String> {
    let helper = privileged_helper()
        .ok_or_else(|| "特权助手未安装（sudo bash Tools/install-polkit.sh）".to_string())?;
    let pkexec = which("pkexec").ok_or_else(|| "系统缺少 pkexec".to_string())?;
    let mut child = Command::new(pkexec)
        .arg(&helper)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("执行特权助手失败: {e}"))?;

    if let Some(seconds) = wait_seconds {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(e) => return Err(format!("执行特权助手失败: {e}")),
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("等待授权超时".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("执行特权助手失败: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let code = output.status.code();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = format!("{}{}", stdout.trim(), stderr.trim());
    Err(match code {
        Some(3) => format!("无法确定虚拟网卡的隧道网关，路由未下发（{detail}）"),
        Some(4) => format!("系统没有 dhclient/dhcpcd，网卡拿不到地址（{detail}）"),
        Some(126) | Some(127) => format!("已取消授权（{detail}）"),
        _ => format!("特权操作失败: {}", if detail.is_empty() { "未知原因".into() } else { detail }),
    })
}

pub fn interface_mtu(iface: &str) -> Option<u16> {
    let text = fs::read_to_string(format!("/sys/class/net/{iface}/mtu")).ok()?;
    text.trim().parse().ok()
}

pub fn interface_ipv4(iface: &str) -> Option<String> {
    let output = Command::new("ip").args(["-4", "-o", "addr", "show", "dev", iface]).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().skip_while(|token| *token != "inet").nth(1).map(|s| s.to_string())
}

pub fn route_exists(net: &str) -> bool {
    let output = Command::new("ip").args(["-4", "route", "show"]).output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().next() == Some(net)),
        Err(_) => false,
    }
}

pub fn routes_on(iface: &str) -> Vec<String> {
    let output = Command::new("ip").args(["-4", "route", "show", "dev", iface]).output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_string))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// 连接时的一次性提权脚本：要 DHCP 地址 → 推导隧道网关 → 下发网段路由 + MTU。
///
/// **必须合并成一次 pkexec**。拆成"先 dhclient 再写路由"两段的话，用户会被弹两次
/// 密码框（每次 pkexec 一次），而且第二次还得在拿到地址之后才知道网关是什么。
/// 这里把网关推导下放进 shell，用同一次提权把三件事一起做完。
///
/// `need_lease` 由调用方根据连接前的网卡状态决定：已经有 IPv4 就别重复要地址。
pub fn connect_routes_script(
    iface: &str,
    cfg: &RouteConfig,
    need_lease: bool,
) -> Result<String, String> {
    validate_mtu(cfg.mtu)?;
    if iface.is_empty() || !iface.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("网卡名不合法: {iface}"));
    }
    for net in &cfg.subnets {
        validate_subnet(net)?;
    }
    let mut lines = vec!["set -e".to_string()];

    if need_lease {
        if let Some(dhcp) = dhcp_lease_script(iface) {
            // 要地址失败不致命：静态地址环境本来就要不到，让后面的网关推导给出结论
            lines.push(dhcp.replace("set -e", ""));
        }
    }

    // 网关 = 网卡上 proto kernel 那条网段路由的第一跳（192.168.50.19/24 → 192.168.50.1）。
    // 推导放进脚本里，才能和 DHCP 共用同一次提权。
    //
    // 注意 awk 里 `OFS` 必须在 `BEGIN` 设好：写在动作里（`$4=1; OFS="."; print`）时，
    // 字段已经按 `-F.` 拆完、打印用的仍是旧分隔符，输出会是 `192 168 50 1` 而不是点分地址。
    lines.push(format!(
        "GW=$(ip -4 route show dev {iface} | awk '/proto kernel/ && / src / {{print $1; exit}}' | cut -d/ -f1 | awk -F. 'BEGIN{{OFS=\".\"}} NF==4{{$4=1; print}}')"
    ));
    lines.push(
        "if [ -z \"$GW\" ]; then echo 'NO_GATEWAY' >&2; exit 3; fi".to_string(),
    );
    lines.push(format!("ip link set dev {iface} mtu {}", cfg.mtu));
    lines.push(format!("ip -4 route del default dev {iface} 2>/dev/null || true"));
    for net in &cfg.subnets {
        lines.push(format!("ip -4 route replace {net} via \"$GW\" dev {iface}"));
    }
    // 把算出来的网关回显给调用方，失败时要能说清是哪一步出的问题
    lines.push("echo \"GATEWAY=$GW\"".to_string());
    Ok(lines.join("\n"))
}

/// 断开后清理：删掉本应用加过的网段路由，按需清空网卡上的 IPv4 地址。
///
/// `flush_addr` 对应设置项 `routes.dhcp`：地址是连接时 `dhclient -1` 要来的，
/// 而 `-1` 要完就退出、没有常驻进程释放租约，所以要靠这里收尾。留成可关，
/// 是因为有人会把静态地址配在网卡上，那些不该被应用清掉。
pub fn disconnect_routes_script(iface: &str, subnets: &[String], flush_addr: bool) -> Result<String, String> {
    if iface.is_empty() || !iface.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("网卡名不合法: {iface}"));
    }
    let mut lines = vec!["set -e".to_string()];
    for net in subnets {
        validate_subnet(net)?;
        lines.push(format!("ip -4 route del {net} dev {iface} 2>/dev/null || true"));
    }
    if flush_addr {
        lines.push(format!("ip -4 addr flush dev {iface} 2>/dev/null || true"));
    }
    Ok(lines.join("\n"))
}

/// 给虚拟网卡申请 IPv4 地址。
///
/// SoftEther 的 Linux 客户端只建立链路层隧道，**不会替你配 IP**：网卡起来了但
/// 没有地址是常态，路由也就没处可下。这里在连接后主动跑一次 DHCP。
///
/// 系统里可能只有 `dhclient`（Debian 系）或 `dhcpcd`（Arch 系），按存在性挑；
/// 都没有就返回 None，由调用方给出可操作的提示，不猜。
pub fn dhcp_lease_script(iface: &str) -> Option<String> {
    if iface.is_empty() || !iface.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    // dhclient 默认后台常驻；用 -1 只请求一次就退出，避免在托盘应用里留守护进程。
    // 用绝对路径调用：GUI 会话的 PATH 常常不含 /sbin，直接写命令名会 command not found。
    if let Some(dhclient) = find_binary("dhclient") {
        return Some(format!(
            "ip -4 addr flush dev {iface} 2>/dev/null || true\n\
             {} -1 -4 {iface} 2>/dev/null || true\n\
             ip link set dev {iface} up",
            dhclient.display()
        ));
    }
    if let Some(dhcpcd) = find_binary("dhcpcd") {
        return Some(format!(
            "ip -4 addr flush dev {iface} 2>/dev/null || true\n\
             {} -1 -4 -t 15 {iface} 2>/dev/null || true\n\
             ip link set dev {iface} up",
            dhcpcd.display()
        ));
    }
    None
}

/// 本机有没有任何可用的 DHCP 客户端，用来区分「DHCP 失败」与「压根没装」
pub fn has_dhcp_client() -> bool {
    find_binary("dhclient").is_some() || find_binary("dhcpcd").is_some()
}

/// 断开时是否真的还有东西要清理。
///
/// 没有残留路由、网卡上也没 IPv4 时，`disconnect_routes_script` 里的每条命令都是空转，
/// 却仍然要弹一次密码框——用户会以为断开本身需要提权。先查一遍，没活就不提权。
pub fn needs_disconnect_cleanup(iface: &str) -> bool {
    if iface.is_empty() || !iface.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return false;
    }
    if !routes_on(iface).is_empty() {
        return true;
    }
    interface_ipv4(iface).is_some()
}

pub fn path_display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_subnet() {
        assert!(validate_subnet("10.0.0.0/8").is_ok());
        assert!(validate_subnet("10.0.0.0/33").is_err());
        assert!(validate_subnet("10.0.0/8").is_err());
        assert!(validate_subnet("10.0.0.0").is_err());
        assert!(validate_subnet("10.0.0.0/8; rm -rf /").is_err());
    }

    #[test]
    fn rejects_bad_target() {
        assert!(validate_target("10.10.4.160").is_ok());
        assert!(validate_target("hzvpn.tracyenergy.cn").is_ok());
        assert!(validate_target("--help").is_err());
        assert!(validate_target("10.10.4.160; id").is_err());
        assert!(validate_target("").is_err());
    }

    #[test]
    fn script_drops_default_route_and_sets_mtu() {
        let cfg = RouteConfig::default();
        let script = connect_routes_script("vpn_vpn", &cfg, false).unwrap();
        assert!(script.contains("ip -4 route del default dev vpn_vpn"));
        // 网关现在是脚本里算出来的 $GW，不再是 Rust 侧拼死的地址
        assert!(script.contains("route replace 10.0.0.0/8 via \"$GW\" dev vpn_vpn"));
        assert!(script.contains("mtu 1400"));
        // 不需要租约时不能混进 dhclient，否则静态地址环境会被多弹一次授权
        assert!(!script.contains("dhclient"));
    }

    #[test]
    fn connect_script_is_a_single_privileged_step() {
        // 关键约束：DHCP 与路由必须在同一个脚本里，否则用户连接一次要输两遍密码
        let cfg = RouteConfig::default();
        let script = connect_routes_script("vpn_vpn", &cfg, true).unwrap();
        let has_dhcp = script.contains("dhclient") || script.contains("dhcpcd");
        if has_dhcp {
            assert!(script.contains("$GW"), "DHCP 与路由必须在同一次提权里完成");
        }
        assert!(script.contains("NO_GATEWAY"), "取不到网关要能明确报出来");
    }

    #[test]
    fn connect_script_rejects_bad_iface() {
        let cfg = RouteConfig::default();
        assert!(connect_routes_script("vpn; rm -rf /", &cfg, false).is_err());
        assert!(connect_routes_script("", &cfg, false).is_err());
    }

    #[test]
    fn gateway_awk_sets_ofs_in_begin() {
        // OFS 写在动作里是无效的：字段已按 -F. 拆完，打印仍用旧分隔符，
        // 结果会是 `192 168 50 1` 这种不能当网关用的串。必须落在 BEGIN。
        let cfg = RouteConfig::default();
        let script = connect_routes_script("vpn_vpn", &cfg, false).unwrap();
        assert!(script.contains("BEGIN{OFS=\".\"}"), "awk 的 OFS 必须在 BEGIN 里设置");
        assert!(!script.contains("$4=1; OFS="), "OFS 不能在赋值之后才设置");
    }

    #[test]
    fn helper_path_matches_policy_annotation() {
        // Rust 侧的调用路径和 polkit policy 里的 exec.path 必须逐字一致，
        // 否则 pkexec 会退回到默认 action id，规则不匹配 → 又开始弹框，
        // 而且不报错，很难查。这里把两者钉在一起。
        assert_eq!(PRIVILEGED_HELPER, "/usr/libexec/softether-tray/softether-tray-net");
        let policy = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../Tools/cn.softether.tray.net.policy"),
        );
        if let Ok(policy) = policy {
            // 注意匹配带 `<` 的完整标签：文件头注释里也提到了 `exec.path`，
            // 只匹配 `exec.path">` 会先命中注释，断言就变成永远为真。
            let needle = format!("org.freedesktop.policykit.exec.path\">{PRIVILEGED_HELPER}<");
            assert!(policy.contains(&needle), "policy 里的 exec.path 必须等于 PRIVILEGED_HELPER");
        }
    }

    #[test]
    fn finds_binary_outside_path() {
        // /bin/sh 一定存在；即便 PATH 被清空也要能找到（模拟 GUI 会话缺 /sbin 的情况）
        assert!(find_binary("sh").is_some());
        // 名字不存在时不能瞎编一个路径出来
        assert!(find_binary("definitely-not-a-real-binary-xyz").is_none());
    }

    #[test]
    fn dhcp_script_uses_absolute_path() {
        if let Some(script) = dhcp_lease_script("vpn_vpn") {
            // 不能只写命令名：GUI 会话 PATH 里常常没有 /sbin
            assert!(script.contains("dhclient") || script.contains("dhcpcd"));
            let first_cmd = script.lines().find(|l| l.contains("dhclient") || l.contains("dhcpcd")).unwrap();
            assert!(first_cmd.trim_start().starts_with('/'), "DHCP 命令必须是绝对路径: {first_cmd}");
        }
    }

    #[test]
    fn dhcp_script_rejects_bad_iface() {
        assert!(dhcp_lease_script("").is_none());
        assert!(dhcp_lease_script("vpn; rm -rf /").is_none());
    }

    #[test]
    fn disconnect_script_flushes_only_when_asked() {
        let subnets = vec!["10.0.0.0/8".to_string()];
        let with_flush = disconnect_routes_script("vpn_vpn", &subnets, true).unwrap();
        assert!(with_flush.contains("ip -4 route del 10.0.0.0/8 dev vpn_vpn"));
        assert!(with_flush.contains("ip -4 addr flush dev vpn_vpn"));

        // 静态地址场景：只删路由，不动地址
        let keep_addr = disconnect_routes_script("vpn_vpn", &subnets, false).unwrap();
        assert!(keep_addr.contains("ip -4 route del 10.0.0.0/8 dev vpn_vpn"));
        assert!(!keep_addr.contains("addr flush"));
    }

    #[test]
    fn disconnect_script_rejects_bad_iface() {
        let subnets = vec!["10.0.0.0/8".to_string()];
        assert!(disconnect_routes_script("vpn; rm -rf /", &subnets, true).is_err());
        assert!(!needs_disconnect_cleanup("vpn; rm -rf /"));
    }

    #[test]
    fn settings_roundtrip_and_defaults() {
        // 只验证序列化结构，不写真实配置目录；避免测试改掉用户的设置。
        let settings = Settings::default();
        let text = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded.routes.subnets.len(), 3);
        assert!(loaded.notifications);
        assert_eq!(loaded.routes.mtu, 1400);
        assert_eq!(loaded.ui_style, "win11");
        // 老配置文件里没有 uiStyle 字段，缺省也只能是 win11
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.routes.subnets.len(), 3);
        assert_eq!(legacy.ui_style, "win11");
    }

    #[test]
    fn ui_style_normalizes_to_known_values() {
        let mut settings = Settings::default();
        settings.ui_style = "deepin".into();
        settings.normalize_ui_style();
        assert_eq!(settings.ui_style, "deepin");
        // 手改坏的取值收敛成 win11，而不是让整份配置失效
        settings.ui_style = "win95".into();
        settings.normalize_ui_style();
        assert_eq!(settings.ui_style, "win11");
    }
}
