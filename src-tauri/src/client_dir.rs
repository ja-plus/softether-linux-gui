//! SoftEther 客户端目录的发现、判定与解析。
//!
//! 三来源模型（对应设置页的三个选项）：
//!
//! | 来源 | 含义 | 能否失效 |
//! |------|------|----------|
//! | `bundled` | deb 包内置，随本应用安装 | 一定存在（除非被手工删掉） |
//! | `detected` | 扫盘扫到的，用户自己装过 | 会（用户卸载客户端后） |
//! | `custom` | 用户手动指定的任意目录 | 会 |
//!
//! **判定只能看文件是否齐备，不能看目录名。** 用户把官方 tar.gz 解到
//! `~/下载/软件/VPN/` 这种中文嵌套目录里也能用，靠名字猜必然漏。
//!
//! 这个模块不做 IO 重活：扫盘只在「用户点开设置页 / 点刷新」时跑，
//! 每次扫码都有明确的目录数和深度上限，避免在几十万文件的 home 下扫出天际。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::settings::{dirs_home, path_display};

/// deb 包内置的客户端目录。装了本应用就一定有。
pub const BUNDLED_CLIENT_DIR: &str = "/usr/lib/softether-tray/client";

/// 两个路径是不是同一个目录：忽略首尾空白与结尾的 `/`。
///
/// 用户手填 `/usr/lib/softether-tray/client/` 是常事，少一个斜杠就不认
/// 会把内置客户端整条判成"自己指定"。
pub fn same_dir(a: &str, b: &str) -> bool {
    fn norm(value: &str) -> &str {
        value.trim().trim_end_matches('/')
    }
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && a == b
}

/// 内置客户端可能落在的目录：deb 源目录 + 只读 `/usr` 时同步出来的用户副本。
///
/// 两个都要认：用户在「自己指定」里填的可能是其中任何一个，
/// 认不出就会把内置客户端显示成自定义目录。
pub fn bundled_dir_aliases() -> Vec<String> {
    vec![BUNDLED_CLIENT_DIR.to_string(), path_display(&bundled_runtime_dir())]
}

/// 路径是不是内置客户端目录。
pub fn is_bundled_dir(dir: &str) -> bool {
    bundled_dir_aliases().iter().any(|alias| same_dir(dir, alias))
}

/// 判定「这个目录是一个能用的 SoftEther 客户端」所必需的文件。
///
/// `hamcore.se2` 不在其中的话，`vpnclient` 会直接 Fatal Error（实测），
/// 所以它必须参与判定，否则会出现「目录里有可执行文件但根本跑不起来」的假阳性。
const REQUIRED_FILES: [&str; 2] = ["vpnclient", "vpncmd"];
const REQUIRED_DATA: [&str; 1] = ["hamcore.se2"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientSource {
    Bundled,
    Detected,
    Custom,
}

impl ClientSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ClientSource::Bundled => "bundled",
            ClientSource::Detected => "detected",
            ClientSource::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "bundled" => Some(ClientSource::Bundled),
            "detected" => Some(ClientSource::Detected),
            "custom" => Some(ClientSource::Custom),
            _ => None,
        }
    }
}

/// 一个候选目录及其体检结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCandidate {
    /// 目录绝对路径
    pub dir: String,
    pub source: ClientSource,
    /// 是否可用：必需文件齐备
    pub usable: bool,
    /// 不可用时的具体缺项，直接给用户看，不要只说"不可用"
    pub problem: Option<String>,
    /// 客户端版本，从 `vpncmd --version` 里抠出来；取不到就是 None
    pub version: Option<String>,
    /// 该目录是否「能写自己家的配置」，非 root 用户装到 /usr/local 时是 false
    pub writable: bool,
}

/// 体检一个目录：文件是否齐、版本是多少。
///
/// 版本通过跑一次 `vpncmd` 拿。这是唯一可信的来源——目录里没有版本文件，
/// 而且用户可能把 4.44 和 5.0 混着装。跑失败不影响整体判定，只是没有版本号。
pub fn inspect(dir: &Path, source: ClientSource) -> ClientCandidate {
    let mut missing: Vec<&str> = Vec::new();
    for name in REQUIRED_FILES.iter().chain(REQUIRED_DATA.iter()) {
        if !dir.join(name).is_file() {
            missing.push(name);
        }
    }
    let writable = is_dir_writable(dir);
    let usable = missing.is_empty() && writable;
    // 缺文件和目录不可写是两回事，用户要做的动作也不同，别糅成一句
    let problem = if !missing.is_empty() {
        if crate::i18n::is_en() {
            Some(format!("Missing {}", missing.join(", ")))
        } else {
            Some(format!("缺少 {}", missing.join("、")))
        }
    } else if !writable {
        Some(
            crate::i18n::tr(
                "目录不可写，客户端无法写入配置与锁文件（常见于只读的 /usr）",
                "The directory is not writable, so the client cannot write its config and lock file (common when /usr is read-only)",
            )
            .to_string(),
        )
    } else {
        None
    };

    ClientCandidate {
        dir: path_display(dir),
        source,
        usable,
        problem,
        version: if missing.is_empty() { probe_version(dir) } else { None },
        writable,
    }
}

/// 内置客户端在**用户地盘**上的运行副本目录。
///
/// 内置源在 `/usr/lib/softether-tray/client`，而 vpnclient 必须往自己的
/// 安装目录里写配置、日志和锁文件。Deepin/ostree 这类系统的 `/usr` 是只读
/// overlay，root 也写不进去，于是客户端启动时报
/// "Unable to create .../.VPN-xxxx" 却**以 0 退出**——上层看起来一切正常，
/// 实际什么都没做。
///
/// 所以首次使用时把内置文件复制到 XDG 数据目录，用这份跑。源目录可写时
/// （普通 deb 安装）仍然直接用源目录，不多复制一份。
pub fn bundled_runtime_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs_home().map(|home| home.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("softether-tray/client")
}

/// 内置客户端需要的文件。`lang.config` 缺失不算致命，但复制过去能锁中文。
const BUNDLED_FILES: [&str; 4] = ["vpnclient", "vpncmd", "hamcore.se2", "lang.config"];

/// 从 `vpn_client.config` 里静态读出来的一个连接配置。
///
/// 只用来看和跳转，不参与连接本身：真连接还是交给 vpncmd，
/// 所以这里只需要名字、服务器、HUB、网卡这几项展示信息。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticAccount {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub hub: String,
    pub nic: String,
    /// 是否配置了密码。匿名认证的配置连不上，界面要能把这件事说出来
    pub has_password: bool,
}

/// 解析一个客户端目录里的全部连接配置。
///
/// 配置是一个带缩进的声明式文本，结构为
/// `declare AccountList { declare Account0 { declare ClientAuth {...} declare ClientOption {...} } }`。
/// 这里不做通用解析器，只按行抓需要的几个键——格式由客户端自己维护，
/// 万一哪天变了，最坏结果是列表少一项，不会把别的功能带崩。
pub fn accounts_in(dir: &Path) -> Vec<StaticAccount> {
    let Ok(bytes) = std::fs::read(dir.join("vpn_client.config")) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut out: Vec<StaticAccount> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        // `declare AccountList` 也以 "declare Account" 开头，必须要求后面全是数字
        if let Some(rest) = line.strip_prefix("declare Account") {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                out.push(StaticAccount {
                    name: String::new(),
                    host: String::new(),
                    port: 0,
                    hub: String::new(),
                    nic: String::new(),
                    has_password: false,
                });
                continue;
            }
        }
        let Some(current) = out.last_mut() else { continue };
        let Some((kind, body)) = line.split_once(' ') else { continue };
        let body = body.trim();
        match (kind, body.split_once(' ')) {
            ("string", Some(("AccountName", value))) => current.name = value.trim().to_string(),
            ("string", Some(("Hostname", value))) => current.host = value.trim().to_string(),
            ("string", Some(("HubName", value))) => current.hub = value.trim().to_string(),
            ("string", Some(("DeviceName", value))) => current.nic = value.trim().to_string(),
            ("uint", Some(("Port", value))) => {
                current.port = value.trim().parse().unwrap_or(0);
            }
            // `$` 表示空值：没设密码
            ("byte", Some(("HashedPassword", value))) => {
                current.has_password = !value.trim().is_empty() && value.trim() != "$";
            }
            _ => {}
        }
    }
    // 名字为空的块是残缺配置，不列出来
    out.retain(|account| !account.name.is_empty());
    out
}

/// 某个客户端目录里配了几个连接账号。
///
/// 直接读 `vpn_client.config` 数 `string AccountName` 行，**不经过 vpncmd**：
/// 同一时刻只有一个客户端实例在跑，其它目录的账号只能用静态方式看。
/// 这个数只用来在界面上回答"账号在哪一份客户端里"，不参与任何连接逻辑。
pub fn account_count(dir: &Path) -> usize {
    let Ok(bytes) = std::fs::read(dir.join("vpn_client.config")) else {
        return 0;
    };
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .filter(|line| line.trim_start().starts_with("string AccountName "))
        .count()
}

/// 找到 `string AccountName <名字>` 所在行的下标（值允许含空格）。
fn account_name_index(lines: &[&str], account: &str) -> Option<usize> {
    lines.iter().position(|line| {
        line.trim()
            .strip_prefix("string AccountName")
            .map(|value| value.trim() == account)
            .unwrap_or(false)
    })
}

/// 按花括号配对找 `declare` 块的收尾 `}` 行下标。
///
/// 起始行本身不带花括号（`declare Xxx` 与 `{` 分两行），必须越过它再判断深度，
/// 否则块当场就被判成结束。
fn block_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        let trimmed = line.trim();
        depth += trimmed.matches('{').count() as i32;
        depth -= trimmed.matches('}').count() as i32;
        if idx > start && depth <= 0 {
            return Some(idx);
        }
    }
    None
}

/// 找到某连接配置 `ClientOption` 块的行区间 `[start, end)`（end 是收尾 `}` 那行）。
///
/// 定位顺序：先找 `string AccountName <名字>` 所在行，
/// 再向前找它所属的 `declare ClientOption`，最后按花括号配对找块尾。
fn client_option_range(lines: &[&str], account: &str) -> Option<(usize, usize)> {
    let name_idx = account_name_index(lines, account)?;
    let start = lines[..name_idx]
        .iter()
        .rposition(|line| line.trim().starts_with("declare ClientOption"))?;
    let end = block_end(lines, start)?;
    Some((start, end))
}

/// 找到某连接配置 `ClientAuth` 块的行区间 `[start, end)`（end 是收尾 `}` 那行）。
///
/// 必须先定位到同一个 `declare AccountN` 块再在块内找 `declare ClientAuth`：
/// 匿名认证的配置没有 ClientAuth 块，直接从账号名往前找会越过块界，
/// 摸到上一个账号的认证块上。
fn client_auth_range(lines: &[&str], account: &str) -> Option<(usize, usize)> {
    let name_idx = account_name_index(lines, account)?;
    let account_start = lines[..name_idx].iter().rposition(|line| {
        line.trim()
            .strip_prefix("declare Account")
            .map(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
    })?;
    let start = lines[account_start..name_idx]
        .iter()
        .position(|line| line.trim().starts_with("declare ClientAuth"))?
        + account_start;
    let end = block_end(lines, start)?;
    Some((start, end))
}

/// 在 `ClientAuth` 块的行里取某个键的值（原样、去掉键后多余空白）。
fn auth_value<'a>(block: &[&'a str], key: &str) -> Option<&'a str> {
    block.iter().find_map(|line| {
        let value = key_value(line.trim(), key)?;
        Some(value.trim_start())
    })
}

/// 行是 `键 值` 时返回值部分（值可以为空），键带后缀（`键X`）不算匹配。
fn key_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?;
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix(' ')
    }
}

/// 读某连接配置的「不使用 UDP 加速」当前取值。
///
/// `vpn_client.config` 里没有这项时是 false（客户端默认使用 UDP 加速）。
/// 读不到文件或找不到该配置时返回 None。
pub fn account_no_udp_accel(dir: &Path, account: &str) -> Option<bool> {
    let text = std::fs::read_to_string(dir.join("vpn_client.config")).ok()?;
    let clean: Vec<&str> = strip_cr(&text);
    let (start, end) = client_option_range(&clean, account)?;
    Some(
        clean[start..end]
            .iter()
            .find_map(|line| {
                let value = line.trim().strip_prefix("bool NoUdpAcceleration")?;
                Some(value.trim() == "true")
            })
            .unwrap_or(false),
    )
}

/// 把「不使用 UDP 加速」写进 `vpn_client.config`。**必须在客户端服务停止时调用**：
/// 运行中的客户端会在退出/改配置时把内存里的整份配置写回，把这里的修改顶掉。
///
/// vpncmd 没有对应命令（`AccountDetailSet` 不含 UDP 参数），改文件是唯一途径。
pub fn set_account_no_udp_accel(dir: &Path, account: &str, enabled: bool) -> Result<(), String> {
    let path = dir.join("vpn_client.config");
    let text = std::fs::read_to_string(&path).map_err(|e| {
        crate::i18n::tr(
            &format!("读取客户端配置失败: {e}"),
            &format!("Cannot read the client config: {e}"),
        )
        .to_string()
    })?;
    let updated = rewrite_no_udp_accel(&text, account, enabled).ok_or_else(|| {
        crate::i18n::tr(
            &format!("客户端配置里找不到连接配置「{account}」"),
            &format!("Profile \"{account}\" was not found in the client config"),
        )
        .to_string()
    })?;
    if updated != text {
        std::fs::write(&path, updated).map_err(|e| {
            crate::i18n::tr(
                &format!("写入客户端配置失败: {e}"),
                &format!("Cannot write the client config: {e}"),
            )
            .to_string()
        })?;
    }
    Ok(())
}

/// 读某连接配置已存的用户名（`ClientAuth` 里的 `string Username`）。
///
/// 口令哈希必须按**这里**的用户名算（`AccountPasswordSet` 也是按已存用户名
/// 哈希的），编辑配置时界面里改了用户名并不会写回配置，按界面值算会算错。
pub fn account_username(dir: &Path, account: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("vpn_client.config")).ok()?;
    let clean: Vec<&str> = strip_cr(&text);
    let (start, end) = client_auth_range(&clean, account)?;
    Some(auth_value(&clean[start..end], "string Username")?.trim().to_string())
}

/// 读某连接配置已存的口令哈希（`ClientAuth` 里的 `byte HashedPassword`），
/// 返回 base64 文本。没设过密码（`$`、空值或没有这一行）时返回 None。
pub fn account_hashed_password(dir: &Path, account: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("vpn_client.config")).ok()?;
    let clean: Vec<&str> = strip_cr(&text);
    let (start, end) = client_auth_range(&clean, account)?;
    let value = auth_value(&clean[start..end], "byte HashedPassword")?.trim();
    if value.is_empty() || value == "$" {
        return None;
    }
    Some(value.to_string())
}

/// 把口令哈希写进 `vpn_client.config`。**必须在客户端服务停止时调用**：
/// 运行中的客户端会在退出/改配置时把内存里的整份配置写回，把这里的修改顶掉。
///
/// 非 ASCII / 超长密码走不了 vpncmd 的交互提示（会把密码改掉），本地算好
/// `HashPassword` 写文件是唯一忠实的途径，见 `accounts.rs` 与 `sha0.rs`。
pub fn set_account_hashed_password(dir: &Path, account: &str, hash_b64: &str) -> Result<(), String> {
    let path = dir.join("vpn_client.config");
    let text = std::fs::read_to_string(&path).map_err(|e| {
        crate::i18n::tr(
            &format!("读取客户端配置失败: {e}"),
            &format!("Cannot read the client config: {e}"),
        )
        .to_string()
    })?;
    let updated = rewrite_hashed_password(&text, account, hash_b64).ok_or_else(|| {
        crate::i18n::tr(
            &format!("客户端配置里找不到连接配置「{account}」的认证块"),
            &format!("The authentication block of profile \"{account}\" was not found in the client config"),
        )
        .to_string()
    })?;
    if updated != text {
        std::fs::write(&path, updated).map_err(|e| {
            crate::i18n::tr(
                &format!("写入客户端配置失败: {e}"),
                &format!("Cannot write the client config: {e}"),
            )
            .to_string()
        })?;
    }
    Ok(())
}

/// 改写配置文本里某连接配置的口令哈希，并把认证方式固定为标准密码验证
/// （`uint AuthType 1`，与 `AccountPasswordSet` 的行为一致）。
/// 返回 None 表示没找到该连接配置的认证块。纯文本变换，单拎出来可测。
fn rewrite_hashed_password(text: &str, account: &str, hash_b64: &str) -> Option<String> {
    fn indent_of(line: &str) -> &str {
        &line[..line.len() - line.trim_start().len()]
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines = strip_cr(text);
    let (start, end) = client_auth_range(&lines, account)?;
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2);
    let mut auth_written = false;
    let mut hash_written = false;
    for (idx, line) in lines.iter().enumerate() {
        if idx > start && idx < end {
            let trimmed = line.trim();
            if key_value(trimmed, "uint AuthType").is_some() {
                out.push(format!("{}uint AuthType 1", indent_of(line)));
                auth_written = true;
                continue;
            }
            if key_value(trimmed, "byte HashedPassword").is_some() {
                out.push(format!("{}byte HashedPassword {hash_b64}", indent_of(line)));
                hash_written = true;
                continue;
            }
        }
        if idx == end && !(auth_written && hash_written) {
            // 原来没有的项插到收尾 `}` 前，缩进和兄弟行保持一致
            let indent = lines[start + 1..end]
                .iter()
                .find(|candidate| !candidate.trim().is_empty())
                .map(|candidate| indent_of(candidate))
                .unwrap_or("\t\t\t\t");
            if !auth_written {
                out.push(format!("{indent}uint AuthType 1"));
            }
            if !hash_written {
                out.push(format!("{indent}byte HashedPassword {hash_b64}"));
            }
        }
        out.push((*line).to_string());
    }
    Some(out.join(eol))
}

/// 按行拆分并去掉 `\r`，统一处理 LF / CRLF 两种换行的配置文件。
fn strip_cr(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// 改写配置文本里的 NoUdpAcceleration，返回 None 表示没找到该连接配置。
/// 纯文本变换，单拎出来可测。
fn rewrite_no_udp_accel(text: &str, account: &str, enabled: bool) -> Option<String> {
    fn indent_of(line: &str) -> &str {
        &line[..line.len() - line.trim_start().len()]
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines = strip_cr(text);
    let (start, end) = client_option_range(&lines, account)?;
    let value = if enabled { "true" } else { "false" };
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut written = false;
    for (idx, line) in lines.iter().enumerate() {
        if idx > start && idx < end && line.trim().starts_with("bool NoUdpAcceleration") {
            out.push(format!("{}bool NoUdpAcceleration {value}", indent_of(line)));
            written = true;
            continue;
        }
        if idx == end && !written {
            // 原来没有这项：插到 ClientOption 收尾 `}` 前，缩进和兄弟行保持一致
            let indent = lines[start + 1..end]
                .iter()
                .find(|candidate| !candidate.trim().is_empty())
                .map(|candidate| indent_of(candidate))
                .unwrap_or("\t\t\t\t");
            out.push(format!("{indent}bool NoUdpAcceleration {value}"));
            written = true;
        }
        out.push((*line).to_string());
    }
    Some(out.join(eol))
}

/// 拿到一个**可写且完整**的内置客户端目录。
///
/// 优先源目录（可写时零成本）；否则把它同步到 `bundled_runtime_dir()`。
/// 返回 `None` 表示内置客户端既不可写、复制也失败了，调用方应报错而不是
/// 假装可用。
pub fn bundled_client_dir() -> Option<PathBuf> {
    let src = PathBuf::from(BUNDLED_CLIENT_DIR);
    if !looks_like_client(&src) {
        return None;
    }
    if is_dir_writable(&src) {
        return Some(src);
    }

    let dst = bundled_runtime_dir();
    std::fs::create_dir_all(&dst).ok()?;
    for name in BUNDLED_FILES {
        let from = src.join(name);
        if !from.is_file() {
            continue;
        }
        let to = dst.join(name);
        // 已经同步过就跳过：这个函数在状态轮询路径上会被反复调用，
        // 每次重抄 8MB 会把界面拖垮。大小相同即认为是最新的
        // （deb 升级会换掉源文件，大小基本必然变化）。
        let up_to_date = std::fs::metadata(&to)
            .ok()
            .zip(std::fs::metadata(&from).ok())
            .map(|(cached, origin)| cached.len() == origin.len())
            .unwrap_or(false);
        if up_to_date {
            continue;
        }
        std::fs::copy(&from, &to).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // 执行位要自己补：copy 继承的是源权限，但副本落在用户目录，
            // 落盘后仍需保证 group/other 可执行不必要，owner 可执行必须有
            if let Ok(meta) = std::fs::metadata(&to) {
                let mut perms = meta.permissions();
                let mode = perms.mode();
                let wanted = if name.ends_with("vpnclient") || name.ends_with("vpncmd") {
                    mode | 0o700
                } else {
                    mode | 0o600
                };
                if wanted != mode {
                    perms.set_mode(wanted);
                    let _ = std::fs::set_permissions(&to, perms);
                }
            }
        }
    }
    looks_like_client(&dst).then_some(dst)
}

fn is_dir_writable(dir: &Path) -> bool {
    // vpnclient 要在这个目录里写自己的配置、日志和锁文件。
    // 装到 /usr/local/vpnclient 的普通用户跑起来会因为写不进去而连不上，
    // 这个信息值得提前告诉用户。
    //
    // **必须真写一次**，不能只看 mode 位：Deepin 这类系统的 /usr 是只读
    // overlay，目录 mode 是 0755（owner 可写），但任何人都写不进去。只看
    // 权限位会把它判成可写，然后 vpnclient 在运行时报
    // "Unable to create /usr/lib/.../.VPN-xxxx" 并**以 0 退出**——上层完全
    // 看不出失败，于是"停止服务"变成了点了没反应。
    let probe = dir.join(".softether-tray-write-probe");
    match std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 从 `vpncmd --version` 的输出里抠版本行。
///
/// 输出形如：`Version 4.44 Build 9807   (Simplified_Chinese)`。
/// 这里只取 `Version X.Y`，Build 号和语言随包不同，不参与比较。
fn probe_version(dir: &Path) -> Option<String> {
    let binary = dir.join("vpncmd");
    let output = std::process::Command::new(&binary)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_version(&text)
}

/// 抽出来单独测：真实二进制在 CI/测试环境不一定有。
pub fn parse_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        // 注意用 `let Some(..) = .. else { continue }` 而不是 `?`：
        // `?` 在首行不是 Version 时会直接结束整个函数，
        // 而实际输出第一行是 "vpncmd 命令 - xxx"，版本在第二行。
        let Some(rest) = line.strip_prefix("Version ") else { continue };
        let Some(version) = rest.split_whitespace().next() else { continue };
        // 形如 4.44 / 5.0 / 4.44.9807，至少要有一个点
        if version.contains('.') && version.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Some(version.to_string());
        }
    }
    None
}

/// 扫盘时允许进入的深度。家目录下 `~/Downloads/vpnclient` 是 2 层，
/// 但有人会解到 `~/下载/软件/softether/vpnclient` 这种 4 层，取 5 留余量。
const SCAN_DEPTH: usize = 5;
/// 扫盘时最多看多少个目录项。防止在超大目录里卡住界面。
const SCAN_BUDGET: usize = 20_000;
/// 这些目录一定不含 SoftEther 客户端，进去纯浪费时间。
const SKIP_DIRS: [&str; 12] = [
    "node_modules", ".git", ".cache", ".local/share/Trash", "vendor", ".cargo", ".rustup",
    ".npm", "__pycache__", ".venv", "site-packages", ".mozilla",
];

struct ScanState {
    budget: usize,
}

impl ScanState {
    fn spend(&mut self) -> bool {
        if self.budget == 0 {
            return false;
        }
        self.budget -= 1;
        true
    }
}

/// 从若干根目录出发，找所有像 SoftEther 客户端的目录。
///
/// 命中后**不再往下走**——客户端目录内部不会有第二个客户端目录，
/// 继续递归只会在 hamcore.se2 解出来的临时目录里浪费时间。
fn scan_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut state = ScanState { budget: SCAN_BUDGET };
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        walk(root, 0, &mut state, &mut found);
        if state.budget == 0 {
            break;
        }
    }
    found
}

fn walk(dir: &Path, depth: usize, state: &mut ScanState, found: &mut Vec<PathBuf>) {
    if depth > SCAN_DEPTH || !state.spend() {
        return;
    }
    if looks_like_client(dir) {
        found.push(dir.to_path_buf());
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // 权限不足的目录直接跳过：家目录里常有 root 所有的目录
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if !state.spend() {
            return;
        }
        let path = entry.path();
        // 不跟随符号链接，否则 ~/Downloads -> /mnt/big 会把整个外置盘拖进来
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if SKIP_DIRS.iter().any(|skip| name == *skip) {
            continue;
        }
        walk(&path, depth + 1, state, found);
    }
}

/// 只做一件事：这个目录里的必需文件齐不齐。
/// 单独的判定函数，方便测试时不依赖真实二进制。
pub fn looks_like_client(dir: &Path) -> bool {
    REQUIRED_FILES.iter().chain(REQUIRED_DATA.iter()).all(|name| dir.join(name).is_file())
}

/// 扫盘用的根目录。
///
/// 只扫用户自己的地盘 + 两个约定俗成的系统位置。**不扫 `/`**：
/// 一次全盘 stat 在机械盘上能干到几十秒，而且设置页会卡死。
fn scan_roots_for_user() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs_home() {
        roots.push(home.join("Downloads"));
        roots.push(home.join("下载"));
        roots.push(home.join("Desktop"));
        roots.push(home.join("桌面"));
        roots.push(home.join("Documents"));
        roots.push(home.join("文档"));
        roots.push(home.join("opt"));
        // 家目录本身放最后：范围最大，前面都没命中时才愿意付这个代价
        roots.push(home);
    }
    roots.push(PathBuf::from("/opt"));
    roots.push(PathBuf::from("/usr/local"));
    roots
}

/// `.deb` / `.rpm` 解包后留下的目录名里带架构，路径没法预知，
/// 只能从 apt 缓存目录里扫。
fn scan_package_cache() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for base in ["/var/cache/apt/archives", "/var/tmp"] {
        let base = PathBuf::from(base);
        if base.is_dir() {
            roots.push(base);
        }
    }
    roots
}

/// 扫盘的全部根目录。
fn all_scan_roots() -> Vec<PathBuf> {
    let mut roots = scan_roots_for_user();
    roots.extend(scan_package_cache());
    roots
}

/// 列出所有候选：内置 + 扫到的 + 用户指定的。
///
/// 顺序即优先级：内置在前（版本确定），扫到的按发现顺序，自定义单独标记。
/// 同一路径只出现一次——用户指定的目录恰好也是扫到的那个时，以后者身份展示。
pub fn candidates(custom_dir: Option<&str>) -> Vec<ClientCandidate> {
    let mut out: Vec<ClientCandidate> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    // 内置项按"实际会用的那个目录"展示：只读 /usr 上用的是同步出来的副本，
    // 判定原始目录会把一个完全能用的客户端标成"缺失"。
    if PathBuf::from(BUNDLED_CLIENT_DIR).is_dir() {
        if let Some(dir) = bundled_client_dir() {
            let candidate = inspect(&dir, ClientSource::Bundled);
            seen.push(candidate.dir.clone());
            out.push(candidate);
        } else {
            let candidate = inspect(&PathBuf::from(BUNDLED_CLIENT_DIR), ClientSource::Bundled);
            seen.push(candidate.dir.clone());
            out.push(candidate);
        }
    }

    let roots = all_scan_roots();
    for dir in scan_roots(&roots) {
        let display = path_display(&dir);
        if seen.contains(&display) {
            continue;
        }
        seen.push(display);
        out.push(inspect(&dir, ClientSource::Detected));
    }

    // 用户指定的目录哪怕文件不全也要列出来 —— 否则用户填错了看不到反馈，
    // 只会觉得"填了没反应"。
    if let Some(custom) = custom_dir.map(str::trim).filter(|s| !s.is_empty()) {
        let path = PathBuf::from(custom);
        let display = path_display(&path);
        if !seen.contains(&display) {
            let mut candidate = inspect(&path, ClientSource::Custom);
            // 目录压根不存在时，inspect 会说"缺少 vpnclient、vpncmd、hamcore.se2"，
            // 但那不是缺文件，是路径错了，得说清楚。
            if !path.is_dir() {
                candidate.usable = false;
                candidate.problem = Some(
                    crate::i18n::tr("目录不存在", "Directory does not exist").to_string(),
                );
                candidate.version = None;
            }
            out.push(candidate);
        }
    }

    out
}

/// 调用方没说来源时按目录反推，别一律记成「自己指定」。
///
/// 内置目录是最要紧的一档：把它记成"自己指定"，界面就会显示"自己指定"
/// 而实际跑的是内置客户端，两边对不上（真踩过：连接配置页切回来就错位）。
/// 扫盘根目录里的完整客户端就是「搜索到的」那一档，其余才算自己指定。
pub fn derive_source(dir: &str) -> ClientSource {
    derive_source_in(dir, &all_scan_roots())
}

/// `derive_source` 的判定层，根目录由调用方给，单拎出来可测。
fn derive_source_in(dir: &str, roots: &[PathBuf]) -> ClientSource {
    if is_bundled_dir(dir) {
        return ClientSource::Bundled;
    }
    let path = Path::new(dir);
    let under_scan_root = roots.iter().any(|root| path.starts_with(root));
    if under_scan_root && looks_like_client(path) {
        return ClientSource::Detected;
    }
    ClientSource::Custom
}

/// 设置里实际要用的那个目录。
///
/// 优先级：用户显式选择 > 内置。用户选的东西失效了（比如卸了自装的客户端）
/// **不回退到内置**，而是返回 None 并带上原因——静默换目录会让用户
/// 以为连的是自己那个客户端，排查起来极其困难。
#[derive(Debug)]
pub struct Resolved {
    pub dir: PathBuf,
    pub source: ClientSource,
    /// 选定的来源失效时的原因，供设置页提示
    pub stale_reason: Option<String>,
}

pub fn resolve(chosen: Option<ClientSource>, custom_dir: Option<&str>) -> Result<Resolved, String> {
    // 没选过（老配置 / 首次运行）：内置优先，这是确定性最高的选项
    let source = chosen.unwrap_or(ClientSource::Bundled);
    match source {
        ClientSource::Bundled => {
            // 只读 /usr 上会自动落到用户目录里的运行副本，所以这里优先用它
            if let Some(dir) = bundled_client_dir() {
                return Ok(Resolved { dir, source, stale_reason: None });
            }
            let dir = PathBuf::from(BUNDLED_CLIENT_DIR);
            if dir.is_dir() {
                Err(if crate::i18n::is_en() {
                    format!(
                        "The built-in client is incomplete ({}); reinstall this app.",
                        dir.display()
                    )
                } else {
                    format!("内置客户端不完整（{}），请重新安装本应用", dir.display())
                })
            } else {
                Err(crate::i18n::tr(
                    "内置客户端目录不存在，请重新安装本应用",
                    "The built-in client directory does not exist; reinstall this app.",
                )
                .into())
            }
        }
        ClientSource::Custom => {
            let raw = custom_dir.map(str::trim).filter(|s| !s.is_empty());
            let raw = raw.ok_or_else(|| {
                crate::i18n::tr(
                    "已选择「自己指定目录」，但没有填路径",
                    "Custom directory is selected but no path was provided.",
                )
                .to_string()
            })?;
            let dir = PathBuf::from(raw);
            if !dir.is_dir() {
                return Err(if crate::i18n::is_en() {
                    format!("The specified directory does not exist: {raw}")
                } else {
                    format!("指定的目录不存在：{raw}")
                });
            }
            if !looks_like_client(&dir) {
                let candidate = inspect(&dir, ClientSource::Custom);
                let problem = candidate.problem.unwrap_or_else(|| {
                    crate::i18n::tr("文件不完整", "files are incomplete").to_string()
                });
                return Err(if crate::i18n::is_en() {
                    format!("{raw} is not a usable SoftEther client directory: {problem}")
                } else {
                    format!("{raw} 不是可用的 SoftEther 客户端目录：{problem}")
                });
            }
            Ok(Resolved { dir, source, stale_reason: None })
        }
        ClientSource::Detected => {
            // detected 只是「用户从扫到的列表里挑了一个」，落到配置里仍然是一个具体路径。
            // 所以它和 custom 一样按路径解析，区别只在措辞和来源标记。
            let raw = custom_dir.map(str::trim).filter(|s| !s.is_empty());
            let raw = raw.ok_or_else(|| {
                crate::i18n::tr(
                    "已选择「搜索到的目录」，但还没有挑选具体目录",
                    "A detected directory is selected but no specific path was chosen.",
                )
                .to_string()
            })?;
            let dir = PathBuf::from(raw);
            if !dir.is_dir() {
                return Err(if crate::i18n::is_en() {
                    format!("The previously selected directory no longer exists: {raw} (the client may have been uninstalled)")
                } else {
                    format!("之前选定的目录已不存在：{raw}（可能已卸载客户端）")
                });
            }
            if !looks_like_client(&dir) {
                let candidate = inspect(&dir, ClientSource::Detected);
                let problem = candidate.problem.unwrap_or_else(|| {
                    crate::i18n::tr("文件不完整", "files are incomplete").to_string()
                });
                return Err(if crate::i18n::is_en() {
                    format!("The previously selected directory is no longer usable: {raw} ({problem})")
                } else {
                    format!("之前选定的目录已不可用：{raw}（{problem}）")
                });
            }
            Ok(Resolved { dir, source, stale_reason: None })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_parsed_from_real_config_shape() {
        // 结构与客户端真实写出来的完全一致：缩进是 tab，
        // 账号字段散落在 ClientOption/ClientAuth 两个子块里
        let dir = std::env::temp_dir().join(format!("softether-tray-parse-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("vpn_client.config"),
            "declare root\r\n{\r\n\tdeclare AccountList\r\n\t{\r\n\t\tdeclare Account0\r\n\t\t{\r\n\t\t\tdeclare ClientAuth\r\n\t\t\t{\r\n\t\t\t\tuint AuthType 1\r\n\t\t\t\tbyte HashedPassword o2+ImbUGISeNwVwztJQkNDkV4C8=\r\n\t\t\t\tstring Username chenja\r\n\t\t\t}\r\n\t\t\tdeclare ClientOption\r\n\t\t\t{\r\n\t\t\t\tstring AccountName tet\r\n\t\t\t\tstring DeviceName VPN\r\n\t\t\t\tstring Hostname hzvpn.example.cn\r\n\t\t\t\tstring HubName tracyhz\r\n\t\t\t\tuint Port 12443\r\n\t\t\t}\r\n\t\t}\r\n\t\tdeclare Account1\r\n\t\t{\r\n\t\t\tdeclare ClientOption\r\n\t\t\t{\r\n\t\t\t\tstring AccountName 带 空格 的名字\r\n\t\t\t\tstring Hostname b.example.cn\r\n\t\t\t\tstring HubName h2\r\n\t\t\t\tuint Port 443\r\n\t\t\t}\r\n\t\t}\r\n\t}\r\n}\r\n",
        )
        .unwrap();
        let list = accounts_in(&dir);
        assert_eq!(list.len(), 2, "AccountList 不能算成一个账号");
        assert_eq!(list[0].name, "tet");
        assert_eq!(list[0].host, "hzvpn.example.cn");
        assert_eq!(list[0].port, 12443);
        assert_eq!(list[0].hub, "tracyhz");
        assert_eq!(list[0].nic, "VPN");
        assert!(list[0].has_password, "有 HashedPassword 就算设过密码");
        assert_eq!(list[1].name, "带 空格 的名字", "名字里可以有空格");
        assert!(!list[1].has_password);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn account_count_reads_config_lines() {
        // 真实配置里每个账号一行 `string AccountName <名字>`，缩进是 4 个 tab；
        // 这里连写入/读取一起验，避免只测解析、实际读不到文件
        let dir = std::env::temp_dir().join(format!("softether-tray-acc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("vpn_client.config");
        std::fs::write(
            &path,
            "declare root\r\n{\r\n\tdeclare AccountList\r\n\t{\r\n\t\tdeclare Account0\r\n\t\t{\r\n\t\t\tstring AccountName tet\r\n\t\t}\r\n\t\tdeclare Account1\r\n\t\t{\r\n\t\t\tstring AccountName work\r\n\t\t}\r\n\t}\r\n}\r\n",
        )
        .unwrap();
        assert_eq!(account_count(&dir), 2, "两个账号都应被数到");

        // 没有配置文件的目录必须是 0，而不是沿用别的目录的结果
        let empty = dir.join("empty");
        let _ = std::fs::create_dir_all(&empty);
        assert_eq!(account_count(&empty), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_parsed_from_real_output() {
        let text = "vpncmd 命令 - SoftEther VPN 命令行管理工具\n\
                    Version 4.44 Build 9807   (Simplified_Chinese)\n\
                    Compiled 2025/04/16 04:50:46 by buildsan\n";
        assert_eq!(parse_version(text).as_deref(), Some("4.44"));
    }

    #[test]
    fn version_parsed_from_english_output() {
        // 换 lang.config 后行首仍是 "Version"，所以这个解析是语言无关的
        let text = "Version 5.0 Build 1234   (English)\n";
        assert_eq!(parse_version(text).as_deref(), Some("5.0"));
    }

    #[test]
    fn version_absent_is_none() {
        // 不能把描述里的 "Version" 当成版本号
        assert_eq!(parse_version("SoftEther VPN Version 描述\n"), None);
        assert_eq!(parse_version(""), None);
        // 没有点号的不是版本
        assert_eq!(parse_version("Version 4\n"), None);
    }

    #[test]
    fn looks_like_client_requires_hamcore() {
        // 只有可执行文件没有 hamcore.se2 时必须判否：
        // 这种目录跑 vpnclient 会 Fatal Error，光看可执行文件会误判成可用。
        let tmp = std::env::temp_dir().join("setray-test-hamcore");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("vpnclient"), b"x").unwrap();
        std::fs::write(tmp.join("vpncmd"), b"x").unwrap();
        assert!(!looks_like_client(&tmp), "缺 hamcore.se2 不能算可用");

        std::fs::write(tmp.join("hamcore.se2"), b"x").unwrap();
        assert!(looks_like_client(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scan_finds_nested_client_and_stops_descending() {
        // 模拟 `~/下载/软件/x/vpnclient` 这种深路径：不能靠目录名猜，必须靠文件判定
        let root = std::env::temp_dir().join("setray-test-scan");
        let _ = std::fs::remove_dir_all(&root);
        let client = root.join("下载/软件/x/vpnclient");
        std::fs::create_dir_all(&client).unwrap();
        for name in ["vpnclient", "vpncmd", "hamcore.se2"] {
            std::fs::write(client.join(name), b"x").unwrap();
        }
        // 客户端目录里再埋一层，验证命中后不再往下走
        let inside = client.join("nested");
        std::fs::create_dir_all(&inside).unwrap();
        for name in ["vpnclient", "vpncmd", "hamcore.se2"] {
            std::fs::write(inside.join(name), b"x").unwrap();
        }

        let found = scan_roots(&[root.clone()]);
        assert_eq!(found.len(), 1, "只应命中外层客户端目录，实际 {:?}", found);
        assert!(found[0].ends_with("vpnclient"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn invalid_custom_dir_reports_path_error_not_missing_files() {
        // 路径不存在时说"缺少 vpnclient"会误导用户去查文件，必须说"目录不存在"
        let list = candidates(Some("/definitely/not/here/xyz"));
        let custom = list.iter().find(|c| c.source == ClientSource::Custom).unwrap();
        assert!(!custom.usable);
        assert_eq!(custom.problem.as_deref(), Some("目录不存在"));
    }

    #[test]
    fn resolve_custom_without_path_is_explicit_error() {
        // 用户选了"自己指定"却没填路径：要报具体原因，不能静默用内置
        let err = resolve(Some(ClientSource::Custom), None).unwrap_err();
        assert!(err.contains("没有填路径"), "实际错误: {err}");
    }

    #[test]
    fn resolve_never_silently_falls_back() {
        // 选定的目录没了就报错。静默回退到内置会让用户以为连的是自己的客户端。
        let err = resolve(Some(ClientSource::Detected), Some("/gone/away")).unwrap_err();
        assert!(err.contains("已不存在"), "实际错误: {err}");
    }

    #[test]
    fn unset_source_defaults_to_bundled() {
        // 老配置里没有 clientSource 字段：默认内置，因为那是唯一确定存在的
        match resolve(None, None) {
            Ok(resolved) => assert_eq!(resolved.source, ClientSource::Bundled),
            // 测试环境若被删了内置目录，报错也必须是"内置"口径，不能换别的来源
            Err(err) => assert!(err.contains("内置"), "实际错误: {err}"),
        }
    }

    #[test]
    fn source_round_trips_through_string() {
        for source in [ClientSource::Bundled, ClientSource::Detected, ClientSource::Custom] {
            assert_eq!(ClientSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(ClientSource::parse("whatever"), None);
    }

    #[test]
    fn same_dir_ignores_trailing_slash_and_whitespace() {
        assert!(same_dir("/usr/lib/softether-tray/client", "/usr/lib/softether-tray/client/"));
        assert!(same_dir(" /usr/lib/softether-tray/client ", "/usr/lib/softether-tray/client"));
        assert!(!same_dir("/usr/lib/softether-tray/clients", "/usr/lib/softether-tray/client"));
        // 空串不能和"什么都不是"判成同一个目录
        assert!(!same_dir("", ""));
        assert!(!same_dir("/", "/"));
    }

    #[test]
    fn bundled_dir_recognizes_both_aliases() {
        assert!(is_bundled_dir(BUNDLED_CLIENT_DIR));
        assert!(is_bundled_dir(&format!("{BUNDLED_CLIENT_DIR}/")));
        // 只读 /usr 时的用户副本同样是内置客户端
        assert!(is_bundled_dir(&path_display(&bundled_runtime_dir())));
        assert!(!is_bundled_dir("/usr/local/vpnclient"));
        assert!(!is_bundled_dir(""));
    }

    #[test]
    fn derive_source_separates_bundled_detected_custom() {
        let root = std::env::temp_dir().join("setray-test-derive");
        let _ = std::fs::remove_dir_all(&root);
        let nested = root.join("dl/vpnclient");
        std::fs::create_dir_all(&nested).unwrap();
        for name in ["vpnclient", "vpncmd", "hamcore.se2"] {
            std::fs::write(nested.join(name), b"x").unwrap();
        }
        let roots = [root.clone()];
        assert_eq!(derive_source_in(BUNDLED_CLIENT_DIR, &roots), ClientSource::Bundled);
        assert_eq!(
            derive_source_in(&path_display(&nested), &roots),
            ClientSource::Detected,
            "扫盘根目录里的完整客户端算「搜索到的」"
        );
        // 不在扫盘范围里的才算自己指定
        assert_eq!(derive_source_in("/srv/whatever/vpnclient", &roots), ClientSource::Custom);
        // 扫盘根目录里但文件不齐的，不会被误标成「搜索到的」
        let broken = root.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        assert_eq!(derive_source_in(&path_display(&broken), &roots), ClientSource::Custom);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 两账号、一个已有 NoUdpAcceleration 一个没有，改写不得串块。
    const UDP_CONFIG: &str = "declare root\r\n\
        {\r\n\
        \tdeclare AccountDatabase\r\n\
        \t{\r\n\
        \t\tdeclare Account0\r\n\
        \t\t{\r\n\
        \t\t\tdeclare ClientAuth\r\n\
        \t\t\t{\r\n\
        \t\t\t\tuint AuthType 1\r\n\
        \t\t\t}\r\n\
        \t\t\tdeclare ClientOption\r\n\
        \t\t\t{\r\n\
        \t\t\t\tstring AccountName tet\r\n\
        \t\t\t\tstring Hostname a.example.cn\r\n\
        \t\t\t\tbool NoUdpAcceleration true\r\n\
        \t\t\t\tuint Port 12443\r\n\
        \t\t\t}\r\n\
        \t\t}\r\n\
        \t\tdeclare Account1\r\n\
        \t\t{\r\n\
        \t\t\tdeclare ClientOption\r\n\
        \t\t\t{\r\n\
        \t\t\t\tstring AccountName 带 空格 的名字\r\n\
        \t\t\t\tstring Hostname b.example.cn\r\n\
        \t\t\t}\r\n\
        \t\t}\r\n\
        \t}\r\n\
        }\r\n";

    #[test]
    fn reads_udp_accel_flag_with_default_false() {
        let dir = std::env::temp_dir().join(format!("softether-tray-udp-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vpn_client.config"), UDP_CONFIG).unwrap();
        assert_eq!(account_no_udp_accel(&dir, "tet"), Some(true));
        // 没写这项 = false（客户端默认开 UDP 加速）
        assert_eq!(account_no_udp_accel(&dir, "带 空格 的名字"), Some(false));
        assert_eq!(account_no_udp_accel(&dir, "不存在"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flips_existing_udp_accel_flag_only_in_target_account() {
        let updated = rewrite_no_udp_accel(UDP_CONFIG, "tet", false).unwrap();
        assert!(updated.contains("bool NoUdpAcceleration false"));
        assert!(!updated.contains("bool NoUdpAcceleration true"));
        // 另一个账号的块不被误伤
        assert!(updated.contains("string AccountName 带 空格 的名字"));
        // CRLF 换行保持原样（客户端按 \r\n 写盘）
        assert!(updated.contains("\r\n"));
        assert!(!updated.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn inserts_udp_accel_flag_when_absent() {
        let updated = rewrite_no_udp_accel(UDP_CONFIG, "带 空格 的名字", true).unwrap();
        assert!(updated.contains("bool NoUdpAcceleration true"));
        // 原来就有那项的账号不受影响（仍是 true，且只出现一次改动）
        let count = updated.matches("bool NoUdpAcceleration").count();
        assert_eq!(count, 2, "两个账号各一项：{updated}");
        // 插入行落在目标 ClientOption 的收尾 } 之前
        let pos_flag = updated.find("string Hostname b.example.cn").unwrap();
        let pos_insert = updated[pos_flag..].find("bool NoUdpAcceleration").unwrap() + pos_flag;
        let pos_close = updated[pos_insert..].find("}").unwrap() + pos_insert;
        assert!(pos_insert < pos_close);
    }

    #[test]
    fn rewrite_reports_unknown_profile() {
        assert!(rewrite_no_udp_accel(UDP_CONFIG, "不存在", true).is_none());
        assert!(rewrite_no_udp_accel("", "tet", true).is_none());
    }

    /// 认证块里已有 HashedPassword 与用户名（LF 换行，顺带覆盖两种换行）。
    const AUTH_CONFIG: &str = "declare root\n\
        {\n\
        \tdeclare AccountDatabase\n\
        \t{\n\
        \t\tdeclare Account0\n\
        \t\t{\n\
        \t\t\tdeclare ClientAuth\n\
        \t\t\t{\n\
        \t\t\t\tuint AuthType 2\n\
        \t\t\t\tbyte HashedPassword OLD_HASH\n\
        \t\t\t\tstring Username chenja\n\
        \t\t\t}\n\
        \t\t\tdeclare ClientOption\n\
        \t\t\t{\n\
        \t\t\t\tstring AccountName tet\n\
        \t\t\t}\n\
        \t\t}\n\
        \t}\n\
        }\n";

    #[test]
    fn reads_username_and_hashed_password() {
        let dir = std::env::temp_dir().join(format!("softether-tray-auth-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vpn_client.config"), AUTH_CONFIG).unwrap();
        assert_eq!(account_username(&dir, "tet").as_deref(), Some("chenja"));
        assert_eq!(account_hashed_password(&dir, "tet").as_deref(), Some("OLD_HASH"));
        assert_eq!(account_username(&dir, "不存在"), None);
        assert_eq!(account_hashed_password(&dir, "不存在"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewrite_hashed_password_replaces_value_and_forces_password_auth() {
        let updated = rewrite_hashed_password(AUTH_CONFIG, "tet", "NEW_HASH").unwrap();
        assert!(updated.contains("byte HashedPassword NEW_HASH"));
        assert!(!updated.contains("OLD_HASH"));
        // 与 AccountPasswordSet 一致：认证方式固定为标准密码验证
        assert!(updated.contains("uint AuthType 1"));
        assert!(!updated.contains("uint AuthType 2"));
        // LF 换行保持原样
        assert!(!updated.contains("\r\n"));
    }

    #[test]
    fn rewrite_hashed_password_inserts_when_absent() {
        // UDP_CONFIG 的 Account0 只有 AuthType：哈希行要插进 ClientAuth 收尾前
        let updated = rewrite_hashed_password(UDP_CONFIG, "tet", "NEWHASH").unwrap();
        assert!(updated.contains("byte HashedPassword NEWHASH"));
        let pos_hash = updated.find("byte HashedPassword").unwrap();
        let pos_close = updated[pos_hash..].find('}').unwrap() + pos_hash;
        assert!(pos_hash < pos_close, "插入行要在认证块收尾 }} 之前");
    }

    #[test]
    fn rewrite_hashed_password_refuses_account_without_auth_block() {
        // Account1 没有 ClientAuth：不能越过块界去改上一个账号的认证块
        assert!(rewrite_hashed_password(UDP_CONFIG, "带 空格 的名字", "X").is_none());
        assert!(rewrite_hashed_password(AUTH_CONFIG, "不存在", "X").is_none());
    }
}
