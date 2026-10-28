use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::settings::{self, Settings, VpncmdAuth};

/// 密码只在本次调用里存在：不写入 settings.json、不进 argv、不落临时文件、
/// 不打印到日志。保存途径见 `save_account` 里的说明。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub hub: String,
    pub user: String,
    pub nic: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default)]
    pub no_udp_accel: bool,
    #[serde(default)]
    pub compress: bool,
    #[serde(default)]
    pub max_tcp: u16,
    #[serde(default)]
    pub half_connection: bool,
}

fn validate_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(crate::i18n::tr(
            "配置名称不能为空，且不能超过 64 个字符",
            "The profile name cannot be empty and must be at most 64 characters.",
        )
        .into());
    }
    if name.starts_with('-') || name.chars().any(|c| c.is_control()) {
        return Err(crate::i18n::tr(
            "配置名称不能以 - 开头，也不能包含控制字符",
            "The profile name cannot start with - or contain control characters.",
        )
        .into());
    }
    Ok(())
}

fn validate_host(host: &str) -> Result<(), String> {
    let host = host.trim();
    if host.is_empty() || host.starts_with('-') || host.len() > 253 {
        return Err(crate::i18n::tr("服务器地址不合法", "Invalid server address.").into());
    }
    if !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':')) {
        return Err(crate::i18n::tr(
            "服务器地址只能包含字母、数字、点、连字符和冒号",
            "The server address may contain only letters, digits, dots, hyphens and colons.",
        )
        .into());
    }
    Ok(())
}

fn validate_hub(hub: &str) -> Result<(), String> {
    let hub = hub.trim();
    if hub.is_empty() || hub.starts_with('-') {
        return Err(crate::i18n::tr(
            "虚拟 HUB 名称不能为空",
            "The virtual HUB name cannot be empty.",
        )
        .into());
    }
    if !hub.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err(crate::i18n::tr(
            "虚拟 HUB 名称只能包含字母、数字、下划线、连字符和点",
            "The virtual HUB name may contain only letters, digits, underscores, hyphens and dots.",
        )
        .into());
    }
    Ok(())
}

fn validate_port(port: u16) -> Result<(), String> {
    if port == 0 {
        return Err(crate::i18n::tr(
            "端口必须在 1-65535 之间",
            "The port must be between 1 and 65535.",
        )
        .into());
    }
    Ok(())
}

pub fn save_account(existing: Option<&str>, input: &AccountInput, settings: &Settings) -> Result<(), String> {
    validate_name(&input.name)?;
    validate_host(&input.host)?;
    validate_hub(&input.hub)?;
    validate_port(input.port)?;
    if input.user.trim().is_empty() {
        return Err(crate::i18n::tr("用户名不能为空", "The user name cannot be empty.").into());
    }
    if settings::find_vpn_dir(settings).is_none() {
        return Err(crate::i18n::tr(
            "找不到 SoftEther VPN Client 目录",
            "Cannot find the SoftEther VPN Client directory.",
        )
        .into());
    }

    let name = input.name.trim();
    let host = input.host.trim();
    let hub = input.hub.trim();
    let user = input.user.trim();
    let nic = input.nic.trim();
    let port = input.port.to_string();

    // 参数名必须和 4.44 客户端一致：AccountCreate 只认
    //   /SERVER:host:port /HUB: /USERNAME: /NICNAME:
    // 以前这里用的是 /HOST: /PORT: /NIC: /ENCRYPT:——那是 AccountGet 的**输出字段名**，
    // 不是命令参数。客户端会逐条回"参数 /HOST 已被指定…不可能指定此参数"，
    // 但**不返回失败关键词**，于是上层只看到"命令没有成功完成"。
    // 加密/压缩也不是参数，而是独立的 AccountEncryptEnable / AccountCompressEnable 命令。
    let server = format!("{host}:{port}");
    let server_arg = format!("/SERVER:{server}");
    let hub_arg = format!("/HUB:{hub}");
    let user_arg = format!("/USERNAME:{user}");
    let nic_arg = format!("/NICNAME:{nic}");

    match existing {
        Some(previous) if previous != name => {
            // 参数必须写成 `/NEW:<值>` 一整段。拆成 "/NEW:" + 值两个 argv 时，
            // vpncmd 会把 "/NEW:" 当成空值、把后面那个值当成多余的位置参数，
            // 回一句"参数 xxx 已被指定…不可能指定此参数"。
            let new_arg = format!("/NEW:{name}");
            let output =
                settings::vpncmd(settings, VpncmdAuth::Plain, &["AccountRename", previous, &new_arg])?;
            if !crate::softether::text_says_success(&output.stdout) {
                return Err(crate::vpn::last_error(&output.stdout));
            }
        }
        _ => {}
    }

    let exists = settings::vpncmd(settings, VpncmdAuth::Plain, &["AccountGet", name])
        .map(|o| crate::softether::text_says_success(&o.stdout))
        .unwrap_or(false);
    if !exists {
        // 用户名和虚拟网卡只有 Create 能一次给全
        let args = ["AccountCreate", name, &server_arg, &hub_arg, &user_arg, &nic_arg];
        let output = settings::vpncmd(settings, VpncmdAuth::Plain, &args)?;
        if !crate::softether::text_says_success(&output.stdout) {
            return Err(crate::vpn::last_error(&output.stdout));
        }
    } else {
        // AccountSet 只支持 /SERVER 与 /HUB，网卡要单独走 AccountNicSet。
        // 用户名不在这里改：口令哈希 = HashPassword(密码, 已存用户名)，改用户名
        // 会让已存哈希失效，而没有明文密码就重算不了（4.44 的 AccountUsernameSet
        // 同样只改名不重算哈希），所以编辑时保持原用户名。
        let args = ["AccountSet", name, &server_arg, &hub_arg];
        let output = settings::vpncmd(settings, VpncmdAuth::Plain, &args)?;
        if !crate::softether::text_says_success(&output.stdout) {
            return Err(crate::vpn::last_error(&output.stdout));
        }
        let args = ["AccountNicSet", name, &nic_arg];
        let output = settings::vpncmd(settings, VpncmdAuth::Plain, &args)?;
        if !crate::softether::text_says_success(&output.stdout) {
            return Err(crate::vpn::last_error(&output.stdout));
        }
    }

    // 加密与压缩是独立命令，显式下发避免继承旧配置
    let crypto: [&str; 2] = [
        if input.encrypted { "AccountEncryptEnable" } else { "AccountEncryptDisable" },
        if input.compress { "AccountCompressEnable" } else { "AccountCompressDisable" },
    ];
    for command in crypto {
        let output = settings::vpncmd(settings, VpncmdAuth::Plain, &[command, name])?;
        if !crate::softether::text_says_success(&output.stdout) {
            return Err(crate::vpn::last_error(&output.stdout));
        }
    }

    // 只在真的改了密码时才下发，空密码表示“保持原密码”，不碰认证方式。
    //
    // 密码保存分两条路，都不进 argv、不落临时文件：
    // - 纯可打印 ASCII 且 ≤ 63 字节：走 vpncmd 的交互提示（不传 /PASSWORD），
    //   从 stdin 连喂两行（「密码」+「确认输入」各一行），不需要重启客户端。
    //   **不能用 /PASSWORD:<值>**：那个值是被原样当密码的，以前传临时文件
    //   路径，存下的密码就是路径字符串，认证必然失败。
    // - 其余（中文、全角、超长）：SoftEther 的 PasswordPrompt 只收 0x20-0x7E
    //   且缓冲 64 字节（Cedar/Console.c），非 ASCII 会被**静默丢弃**、超长被
    //   截断，存下的密码根本不是用户输入的，认证必然失败。这类密码改由本地
    //   算 SoftEther 的 HashPassword（sha0.rs）直接写 vpn_client.config，
    //   与「不使用 UDP 加速」共用一次停服→改文件→起服。
    let mut hashed_password: Option<String> = None;
    if !input.password.is_empty() {
        if prompt_safe(&input.password) {
            let prompt_input = format!("{pw}\n{pw}\n", pw = input.password);
            let output = settings::vpncmd_with_input(
                settings,
                VpncmdAuth::Plain,
                &["AccountPasswordSet", name, "/TYPE:standard"],
                &prompt_input,
            )?;
            if !crate::softether::text_says_success(&output.stdout) {
                return Err(crate::vpn::last_error(&output.stdout));
            }
        } else {
            let dir = settings::find_vpn_dir(settings).ok_or_else(|| {
                crate::i18n::tr(
                    "找不到 SoftEther VPN Client 目录",
                    "Cannot find the SoftEther VPN Client directory.",
                )
                .to_string()
            })?;
            // 与 AccountPasswordSet 一致：按配置里**已存**的用户名算哈希。
            // 编辑配置时界面里改用户名并不会写回配置，按界面值算会算错。
            let username = crate::client_dir::account_username(&dir, name)
                .unwrap_or_else(|| user.to_string());
            hashed_password = Some(crate::sha0::hash_password_b64(&username, &input.password));
        }
    }

    // 只保留官方 4.44 客户端确实支持的参数。实测 /NOQOS 单独用会返回成功但实际生效的是
    // 全量参数形式，所以这里始终把所有项一次给全，避免漏项被当成“已设置”。
    let tcp = if input.max_tcp == 0 { 1 } else { input.max_tcp.clamp(1, 32) }.to_string();
    // /MAXTCP 也要拼成一整段；拆开时客户端会回"TCP 连接请在 1-32 之间选定"，
    // 看着像值越界，其实是它根本没收到值。
    let max_tcp_arg = format!("/MAXTCP:{tcp}");
    let detail: Vec<&str> = vec![
        "AccountDetailSet",
        name,
        &max_tcp_arg,
        "/INTERVAL:1",
        "/TTL:0",
        if input.half_connection { "/HALF:yes" } else { "/HALF:no" },
        "/BRIDGE:no",
        "/MONITOR:no",
        "/NOTRACK:no",
        "/NOQOS:no",
    ];
    let output = settings::vpncmd(settings, VpncmdAuth::Plain, &detail)?;
    if !crate::softether::text_says_success(&output.stdout) {
        return Err(crate::vpn::last_error(&output.stdout));
    }

    // 「不使用 UDP 加速」与（必要时）口令哈希都只能改配置文件（会短暂重启客户端）
    apply_client_config_edits(settings, name, hashed_password.as_deref(), input.no_udp_accel)?;

    Ok(())
}

/// vpncmd 交互提示（`PasswordPrompt`，Cedar/Console.c）只接受 0x20-0x7E 的
/// 可打印 ASCII，且缓冲只有 64 字节：非 ASCII 会被静默丢弃、超长会被截断。
/// 这类密码不能走提示路径，见 `save_account`。
fn prompt_safe(password: &str) -> bool {
    password.len() <= 63 && password.bytes().all(|b| (0x20..=0x7E).contains(&b))
}

/// 把只能写配置文件的两项——「不使用 UDP 加速」与（非 ASCII / 超长密码的）
/// 口令哈希——一次性写进客户端配置并让客户端重新加载。
///
/// vpncmd 没有这两项的忠实途径（`AccountDetailSet` 不含 UDP 参数；口令走
/// 交互提示会丢非 ASCII），改 `vpn_client.config` 是唯一途径；而运行中的
/// 客户端会在退出/改配置时把内存里的整份配置写回、把修改顶掉，所以要
/// 停服务 → 改文件 → 起服务。只有取值真的变化时才做这套动作，
/// 否则每次保存配置都要白白断一次线。
fn apply_client_config_edits(
    settings: &Settings,
    name: &str,
    hashed_password: Option<&str>,
    no_udp_accel: bool,
) -> Result<(), String> {
    let dir = settings::find_vpn_dir(settings).ok_or_else(|| {
        crate::i18n::tr(
            "找不到 SoftEther VPN Client 目录",
            "Cannot find the SoftEther VPN Client directory.",
        )
        .to_string()
    })?;
    let password_changed = match hashed_password {
        Some(hash) => crate::client_dir::account_hashed_password(&dir, name).as_deref() != Some(hash),
        None => false,
    };
    let udp_changed = crate::client_dir::account_no_udp_accel(&dir, name).unwrap_or(false) != no_udp_accel;
    if !password_changed && !udp_changed {
        return Ok(());
    }
    let was_running = crate::vpn::service_running();
    if was_running {
        crate::vpn::stop_service(settings, Some(60))?;
    }
    // root 跑的客户端会把配置写成 root:root 0600：改不动时先请特权助手
    // 把属主交还给当前用户（claim-config），再重试一次。
    if write_client_config(&dir, name, hashed_password, no_udp_accel).is_err() {
        let _ = settings::run_helper_with_timeout(
            &["claim-config", &dir.to_string_lossy()],
            Some(5),
        );
        write_client_config(&dir, name, hashed_password, no_udp_accel)?;
    }
    if was_running && crate::vpn::start_service(settings)? {
        crate::vpn::mark_managed();
    }
    Ok(())
}

fn write_client_config(
    dir: &std::path::Path,
    name: &str,
    hashed_password: Option<&str>,
    no_udp_accel: bool,
) -> Result<(), String> {
    if let Some(hash) = hashed_password {
        crate::client_dir::set_account_hashed_password(dir, name, hash)?;
    }
    crate::client_dir::set_account_no_udp_accel(dir, name, no_udp_accel)
}

pub fn delete_account(settings: &Settings, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let output = settings::vpncmd(settings, VpncmdAuth::Plain, &["AccountDelete", name])?;
    if crate::softether::text_says_success(&output.stdout) {
        Ok(())
    } else {
        Err(crate::vpn::last_error(&output.stdout))
    }
}

/// 虚拟网卡操作：建 / 删 / 启停。
///
/// **先按普通权限跑，只有撞上权限错误才提权**。实测 4.44 客户端在普通用户下
/// 就能建、删、启停虚拟网卡（真正要 root 的是给网卡配地址那一步，那走特权助手）。
/// 一上来就用 pkexec，等于每次管理网卡都白白弹一次授权框。
fn nic_command(settings: &Settings, command: &str, name: &str) -> Result<(), String> {
    let plain = settings::vpncmd(settings, VpncmdAuth::Plain, &[command, name])?;
    if crate::softether::text_says_success(&plain.stdout) {
        return Ok(());
    }
    let error = crate::vpn::last_error(&plain.stdout);
    // 名称重复、网卡正被占用这类错误提权也没用，直接报给用户
    if !looks_like_permission_error(&error) {
        return Err(error);
    }
    let elevated = settings::vpncmd(settings, VpncmdAuth::Elevated, &[command, name])?;
    if crate::softether::text_says_success(&elevated.stdout) {
        Ok(())
    } else {
        Err(crate::vpn::last_error(&elevated.stdout))
    }
}

fn looks_like_permission_error(message: &str) -> bool {
    ["权限", "管理员", "Permission", "permission", "denied", "Access", "root"]
        .iter()
        .any(|hint| message.contains(hint))
}

pub fn create_nic(settings: &Settings, name: &str) -> Result<(), String> {
    validate_name(name)?;
    nic_command(settings, "NicCreate", name)
}

pub fn delete_nic(settings: &Settings, name: &str) -> Result<(), String> {
    validate_name(name)?;
    nic_command(settings, "NicDelete", name)
}

pub fn set_nic_enabled(settings: &Settings, name: &str, enabled: bool) -> Result<(), String> {
    validate_name(name)?;
    nic_command(settings, if enabled { "NicEnable" } else { "NicDisable" }, name)
}

pub fn ping(host: &str, count: u32) -> Result<crate::softether::DiagResult, String> {
    settings::validate_target(host)?;
    let count = count.clamp(1, 100).to_string();
    let output = Command::new("ping")
        .args(["-c", &count, "-i", "0.5", "-W", "1", host])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            crate::i18n::tr(
                &format!("无法执行 ping: {e}"),
                &format!("Cannot run ping: {e}"),
            )
            .to_string()
        })?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(crate::softether::parse_ping(host, &text))
}
