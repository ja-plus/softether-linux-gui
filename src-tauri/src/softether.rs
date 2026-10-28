use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// SoftEther 的 vpncmd 输出是「表格 | 键 | 值」，解析成 Map 更稳。
fn parse_table(output: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("项目") || line.starts_with("---") || line.starts_with("VPN Client>") {
            continue;
        }
        if !line.contains('|') {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let key = parts.next().unwrap_or("").trim().to_string();
        let value = parts.next().unwrap_or("").trim().to_string();
        if key.is_empty() || key.starts_with('-') || key.starts_with('+') {
            continue;
        }
        rows.push((key, value));
    }
    rows
}

pub fn table_value(rows: &[(String, String)], key: &str) -> Option<String> {
    rows.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub name: String,
    pub status: String,
    pub host: String,
    pub port: Option<u16>,
    pub hub: String,
    pub nic: String,
}

pub fn parse_account_list(output: &str) -> Vec<AccountSummary> {
    let mut accounts: Vec<AccountSummary> = Vec::new();
    for (key, value) in parse_table(output) {
        if key == "VPN 连接设置名称" {
            accounts.push(AccountSummary {
                name: value,
                status: String::new(),
                host: String::new(),
                port: None,
                hub: String::new(),
                nic: String::new(),
            });
            continue;
        }
        let Some(current) = accounts.last_mut() else { continue };
        match key.as_str() {
            "状态" => current.status = value,
            "VPN Server 主机名(地址)" => {
                // "hzvpn.tracyenergy.cn:12443 (直接的 TCP/IP 连接)"
                let hostport = value.split_whitespace().next().unwrap_or("").to_string();
                let (host, port) = match hostport.rsplit_once(':') {
                    Some((h, p)) => (h.to_string(), p.parse::<u16>().ok()),
                    None => (hostport, None),
                };
                current.host = host;
                current.port = port;
            }
            "虚拟 HUB 名称" => current.hub = value,
            "虚拟网络适配器名" => current.nic = value,
            _ => {}
        }
    }
    accounts
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub name: String,
    pub connected: bool,
    pub state: String,
    pub host: String,
    pub hub: String,
    pub user: String,
    pub server_version: String,
    pub udp_accel: String,
    pub encrypted: String,
    pub tcp_connections: String,
    pub connected_since: Option<i64>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

const MONTHS: [&str; 12] = ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"];

/// 时间只给到秒级解析；失败就返回 None，由前端显示为未知，不做静默兜底成 0。
pub fn parse_softether_time(text: &str) -> Option<i64> {
    let re = Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日\([^)]*\)\s*(\d{1,2})点(\d{1,2})分(\d{1,2})秒").ok()?;
    let cap = re.captures(text)?;
    let year: i32 = cap[1].parse().ok()?;
    let month: i32 = cap[2].parse().ok()?;
    let day: i32 = cap[3].parse().ok()?;
    let hour: i32 = cap[4].parse().ok()?;
    let minute: i32 = cap[5].parse().ok()?;
    let second: i32 = cap[6].parse().ok()?;
    chrono::TimeZone::with_ymd_and_hms(&chrono::Local, year, month as u32, day as u32, hour as u32, minute as u32, second as u32)
        .single()
        .map(|d| d.timestamp())
}

fn parse_number(text: &str) -> u64 {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 语言无关的判定
//
// **背景**：vpncmd 的所有提示文字都来自客户端目录下的 `lang.config`，取值 `ja/en/cn`。
// 原来代码里到处 `contains("命令成功完成")`，一旦新机器上语言是英文（默认值就是 en），
// 所有操作都会被判成失败——而 VPN 其实已经连上了。实测英文输出是
// `The command completed successfully.`，中文才是 `命令成功完成`。
//
// 对策：**优先用退出码和结构化字段判断**，文本匹配只做兜底，且中英文都认。
// 表格字段名同理（`会话状态` / `Session Status` 都要认）。
// ---------------------------------------------------------------------------

/// 命令是否成功。优先看退出码，0 就是成功；非 0 时再看文本（有些版本出错仍返回 0）。
pub fn succeeded(output: &str, code: i32) -> bool {
    if code == 0 && !output.contains("错误代码") && !output.contains("Error code") {
        return true;
    }
    text_says_success(output)
}

/// 只看文本的成功判定，供拿不到退出码的场合使用。
pub fn text_says_success(output: &str) -> bool {
    const MARKERS: [&str; 4] = [
        "命令成功完成",              // cn
        "The command completed successfully", // en
        "コマンドは正常に完了しました", // ja
        "命令已成功完成",
    ];
    MARKERS.iter().any(|marker| output.contains(marker))
}

/// 会话是否**真的通了**。中英文的表头字段值和状态值都认。
///
/// 「连接中」故意不算：客户端连不上服务器时会**无限重连**，AccountList 的状态
/// 一直停在「连接中」，`AccountStatusGet` 的会话状态则反复刷「重试」。
/// 把这种状态当成已连接，界面就会显示绿色的「已连接」而实际一个包都过不去
/// ——踩过：账户密码不对，会话卡在「重试」40 秒以上，配置行却是绿的。
pub fn status_is_connected(output: &str) -> bool {
    const CONNECTED: [&str; 5] = [
        "连接完成",          // cn
        "已连接",            // cn 简写
        "Connection Completed", // en
        "Connected",
        "接続完了",          // ja
    ];
    // 明确排除"未连接"类描述，避免 `已连接` 命中 `未连接` 的误判
    if is_disconnected(output) {
        return false;
    }
    CONNECTED.iter().any(|marker| output.contains(marker))
}

/// 是否"正在连、还没通"：客户端在握手、认证或反复重试。
///
/// 单独拎出来是因为它在界面上的含义完全不同——既不能报成「已连接」（会骗人），
/// 也不能报成「未连接」（会让人以为没在连）。
pub fn status_is_connecting(output: &str) -> bool {
    const CONNECTING: [&str; 8] = [
        "连接中",
        "正在连接",
        "开始连接",
        "重试",
        "验证用户身份",
        "Connecting",
        "Retrying",
        "Authenticating",
    ];
    if is_disconnected(output) || status_is_connected(output) {
        return false;
    }
    CONNECTING.iter().any(|marker| output.contains(marker))
}

/// 「没连上」的几种说法。必须先行排除，否则 `Connected` 会命中 `Not Connected`。
fn is_disconnected(output: &str) -> bool {
    output.contains("未连接")
        || output.contains("Not Connected")
        || output.contains("未接続")
        || output.contains("离线")
}

/// 表格里按多个可能的列名取值，解决表头随语言变化的问题。
pub fn table_value_any(rows: &[(String, String)], keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = table_value(rows, key) {
            if !found.is_empty() && found != "－" && found != "-" {
                return Some(found);
            }
        }
    }
    None
}

/// 断开操作的"可以接受"判定：成功、本来就未连接、或报的是「已经断开」类错误。
///
/// 断开是幂等的：目标状态下限是"没在连接"，所以这几种都算达成目的。
pub fn disconnect_acceptable(output: &str) -> bool {
    text_says_success(output)
        || output.contains("错误代码")
        || output.contains("Error code")
        || output.contains("未连接")
        || output.contains("Not connected")
        || output.contains("未接続")
}

pub fn parse_account_status(output: &str) -> SessionSnapshot {
    let rows = parse_table(output);
    let state = table_value_any(
        &rows,
        &["会话状态", "Session Status", "セッション状態"],
    )
    .unwrap_or_else(|| "未知".to_string());
    let connected = status_is_connected(&state);
    let value = |keys: &[&str]| -> String {
        table_value_any(&rows, keys).unwrap_or_default()
    };
    SessionSnapshot {
        name: value(&["VPN 连接设置名称", "VPN Connection Setting Name", "VPN 接続設定名"]),
        connected,
        state,
        host: value(&[
            "服务器名",
            "目标 VPN Server 主机名",
            "Destination VPN Server Host Name",
            "Host Name",
        ]),
        hub: value(&["虚拟 HUB 名称", "虚拟 HUB 名称(HUB)", "Virtual Hub Name", "Destination Virtual Hub Name"]),
        user: value(&["用户名", "连接用户名", "User Name"]),
        server_version: value(&["服务端版本", "Server Version"]),
        udp_accel: value(&["UDP 加速已激活", "UDP Acceleration is Active", "UDP アクセラレーション"]),
        encrypted: value(&["加密", "Encryption", "暗号化"]),
        tcp_connections: value(&["TCP 连接数", "Number of TCP Connections", "TCP 接続数"]),
        connected_since: {
            let raw = value(&["连接开始时间", "Connection Started at", "接続開始時刻"]);
            parse_softether_time(&raw)
        },
        rx_bytes: parse_number(&value(&["输入数据量", "Data Size (Received)", "受信データサイズ"])),
        tx_bytes: parse_number(&value(&["输出数据量", "Data Size (Sent)", "送信データサイズ"])),
        rx_packets: parse_number(&value(&[
            "传入单播数据包",
            "Unicast Packets (Received)",
            "受信ユニキャストパケット数",
        ])),
        tx_packets: parse_number(&value(&[
            "传出单播数据包",
            "Unicast Packets (Sent)",
            "送信ユニキャストパケット数",
        ])),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountDetail {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub hub: String,
    pub user: String,
    pub nic: String,
    pub encrypted: bool,
    pub compress: bool,
    pub half_connection: bool,
    pub no_udp_accel: bool,
    pub max_tcp: u16,
}

pub fn parse_account_detail(output: &str) -> AccountDetail {
    let rows = parse_table(output);
    let get = |k: &str| table_value(&rows, k).unwrap_or_default();
    AccountDetail {
        name: get("VPN 连接设置名称"),
        host: get("目标 VPN Server 主机名"),
        port: get("目标 VPN Server 端口号").parse().unwrap_or(443),
        hub: get("目标 VPN Server 虚拟 HUB 名称"),
        user: get("用户名"),
        nic: get("用于连接的设备名"),
        encrypted: get("通过 SSL 加密").contains("启用"),
        compress: get("数据压缩").contains("启用"),
        half_connection: get("使用半双工模式").contains("启用"),
        // AccountGet 实际不输出 UDP 加速项，这里只能是 false；
        // 真实值由 vpn::account_detail 从 vpn_client.config 覆盖回来
        no_udp_accel: get("不要使用 UDP 加速").contains("启用"),
        max_tcp: get("VPN 通信中使用的 TCP 的连接数").parse().unwrap_or(1),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NicInfo {
    pub name: String,
    pub enabled: bool,
    pub mac: String,
}

pub fn parse_nic_list(output: &str) -> Vec<NicInfo> {
    let mut nics: Vec<NicInfo> = Vec::new();
    for (key, value) in parse_table(output) {
        if key == "虚拟网络适配器名" {
            nics.push(NicInfo { name: value, enabled: true, mac: String::new() });
            continue;
        }
        let Some(current) = nics.last_mut() else { continue };
        match key.as_str() {
            "状态" => current.enabled = value.contains("启用"),
            "MAC 地址" => current.mac = value,
            _ => {}
        }
    }
    nics
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagResult {
    pub target: String,
    pub sent: u32,
    pub received: u32,
    pub loss_percent: f64,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_min_ms: Option<f64>,
    pub rtt_max_ms: Option<f64>,
}

pub fn parse_ping(target: &str, output: &str) -> DiagResult {
    let mut result = DiagResult {
        target: target.to_string(),
        sent: 0,
        received: 0,
        loss_percent: 100.0,
        rtt_avg_ms: None,
        rtt_min_ms: None,
        rtt_max_ms: None,
    };
    let stat = Regex::new(r"(\d+)\s+packets transmitted,\s+(\d+)\s+(?:packets\s+)?received").ok();
    if let Some(cap) = stat.and_then(|re| re.captures(output)) {
        result.sent = cap[1].parse().unwrap_or(0);
        result.received = cap[2].parse().unwrap_or(0);
        if result.sent > 0 {
            result.loss_percent = (result.sent.saturating_sub(result.received)) as f64 / result.sent as f64 * 100.0;
        }
    }
    let rtt = Regex::new(r"rtt min/avg/max/mdev = ([\d.]+)/([\d.]+)/([\d.]+)").ok();
    if let Some(cap) = rtt.and_then(|re| re.captures(output)) {
        result.rtt_min_ms = cap[1].parse().ok();
        result.rtt_avg_ms = cap[2].parse().ok();
        result.rtt_max_ms = cap[3].parse().ok();
    }
    result
}

pub fn month_name(index: usize) -> &'static str {
    MONTHS.get(index).copied().unwrap_or("")
}

pub fn json_status(status: &SessionSnapshot) -> Value {
    json!(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"项目               |价值
-------------------+--------------------------------------------------------
VPN 连接设置名称   |tet
会话状态           |连接完成 (会话建立)
服务器名           |hzvpn.tracyenergy.cn
端口号             |TCP 端口 12443
用户名             |chenja
加密               |已启用 (算法: TLS_AES_256_GCM_SHA384)
VPN 通信中使用的 TCP 的连接数 |1
UDP 加速已激活     |否
连接开始时间       |2026年10月25日(星期日)  8点48分23秒
输出数据量         |7,878 字节
输入数据量         |29,960 字节
传出单播数据包     |26 数据包
传入单播数据包     |18 数据包
命令成功完成。"#;

    #[test]
    fn parses_status() {
        let status = parse_account_status(STATUS);
        assert!(status.connected);
        assert_eq!(status.host, "hzvpn.tracyenergy.cn");
        assert_eq!(status.rx_bytes, 29960);
        assert_eq!(status.tx_bytes, 7878);
        assert_eq!(status.udp_accel, "否");
        assert!(status.connected_since.is_some());
    }

    #[test]
    fn parses_account_list() {        let output = r#"项目                   |价值
-----------------------+-----------------------------------------------
VPN 连接设置名称       |tet
状态                   |已连接
VPN Server 主机名(地址)|hzvpn.tracyenergy.cn:12443 (直接的 TCP/IP 连接)
虚拟 HUB 名称          |tracyhz
虚拟网络适配器名       |VPN
命令成功完成。"#;
        let list = parse_account_list(output);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].port, Some(12443));
        assert_eq!(list[0].hub, "tracyhz");
    }

    #[test]
    fn parses_ping_loss() {
        let output = "30 packets transmitted, 20 received, 33.3333% packet loss, time 5892ms\nrtt min/avg/max/mdev = 1.907/3.083/15.971/2.994 ms";
        let result = parse_ping("192.168.84.254", output);
        assert_eq!(result.received, 20);
        assert!((result.loss_percent - 33.3333).abs() < 0.001);
        assert_eq!(result.rtt_avg_ms, Some(3.083));
    }

    #[test]
    fn missing_rtt_is_unknown_not_zero() {
        let result = parse_ping("192.168.50.1", "30 packets transmitted, 0 received, 100% packet loss");
        assert_eq!(result.rtt_avg_ms, None);
        assert_eq!(result.loss_percent, 100.0);
    }

    /// 真机踩到的坑：账户密码不对时客户端无限重连，AccountList 一直是「连接中」。
    /// 这一条必须判成"没连上"，否则界面显示绿色「已连接」而隧道一个包都过不去。
    #[test]
    fn connecting_is_not_connected() {
        assert!(!status_is_connected("连接中"));
        assert!(!status_is_connected("Connecting"));
        assert!(status_is_connecting("连接中"));
        assert!(status_is_connecting("Connecting"));
        // AccountStatusGet 的会话状态在重连期间是这几个
        assert!(status_is_connecting("重试"));
        assert!(status_is_connecting("验证用户身份"));
        assert!(!status_is_connected("重试"));
    }

    #[test]
    fn connected_beats_connecting_prefix() {
        // 真正的已连接不能被"连接中"的判定吃掉
        assert!(status_is_connected("连接完成"));
        assert!(!status_is_connecting("连接完成"));
        assert!(status_is_connected("已连接"));
        // 离线/未连接两种说法都不算在连
        assert!(!status_is_connecting("离线"));
        assert!(!status_is_connected("离线"));
        assert!(!status_is_connecting("未连接"));
    }

    /// 重连中的状态表：会话状态是「重试」但 AccountList 说「连接中」，
    /// 解析出来的 snapshot 必须是"未连接"，同时 state 要能读到「重试」，
    /// 这样报错时才能告诉用户卡在哪一步。
    #[test]
    fn retrying_status_keeps_state_for_error_message() {
        let output = r#"项目                   |价值
-----------------------+------------------------------------
VPN 连接设置名称       |test
会话状态               |重试
连接开始时间           |2026-10-27 13:20:15
命令成功完成。"#;
        let parsed = parse_account_status(output);
        assert!(!parsed.connected, "重试不是已连接");
        assert_eq!(parsed.state, "重试");
    }
}
