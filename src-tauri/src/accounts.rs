use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::settings::{self, Settings, VpncmdAuth};

/// 密码只在本次调用里存在：先写一个 0600 的临时文件，用完立刻删除。
/// 不写入 settings.json，也不放进 URL、不打印到日志。
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

fn with_password_file<T>(password: &str, action: impl FnOnce(&str) -> Result<T, String>) -> Result<T, String> {
    let dir = std::env::temp_dir();
    let path: PathBuf = dir.join(format!("softether-tray-pw-{}.txt", std::process::id()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| {
            crate::i18n::tr(
                &format!("无法创建临时文件: {e}"),
                &format!("Cannot create a temporary file: {e}"),
            )
            .to_string()
        })?;
        file.write_all(password.as_bytes()).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        drop(file);
        action(&path.to_string_lossy())
    })();
    let _ = fs::remove_file(&path);
    result
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

    let mut common: Vec<&str> = vec!["/HOST:", host, "/PORT:", &port, "/HUB:", hub, "/NIC:", nic];
    // 连接加密是独立开关；这里显式给出，避免继承旧配置。
    common.push(if input.encrypted { "/ENCRYPT:yes" } else { "/ENCRYPT:no" });

    match existing {
        Some(previous) if previous != name => {
            let output = settings::vpncmd(settings, VpncmdAuth::Plain, &["AccountRename", previous, "/NEW:", name])?;
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
        let mut args: Vec<&str> = vec!["AccountCreate", name];
        args.extend(common.iter().copied());
        let output = settings::vpncmd(settings, VpncmdAuth::Plain, &args)?;
        if !crate::softether::text_says_success(&output.stdout) {
            return Err(crate::vpn::last_error(&output.stdout));
        }
    }

    let mut set_args: Vec<&str> = vec!["AccountSet", name];
    set_args.extend(common.iter().copied());
    let output = settings::vpncmd(settings, VpncmdAuth::Plain, &set_args)?;
    if !crate::softether::text_says_success(&output.stdout) {
        return Err(crate::vpn::last_error(&output.stdout));
    }

    if !input.password.is_empty() {
        // 只在真的改了密码时才下发，空密码表示“保持原密码”，不碰认证方式。
        with_password_file(&input.password, |path| {
            let output = settings::vpncmd(
                settings,
                VpncmdAuth::Plain,
                &["AccountPasswordSet", name, "/PASSWORD:", path, "/TYPE:standard"],
            )?;
            if crate::softether::text_says_success(&output.stdout) {
                Ok(())
            } else {
                Err(crate::vpn::last_error(&output.stdout))
            }
        })?;
    }

    // 只保留官方 4.44 客户端确实支持的参数。实测 /NOQOS 单独用会返回成功但实际生效的是
    // 全量参数形式，所以这里始终把所有项一次给全，避免漏项被当成“已设置”。
    let tcp = if input.max_tcp == 0 { 1 } else { input.max_tcp.clamp(1, 32) }.to_string();
    let detail: Vec<&str> = vec![
        "AccountDetailSet",
        name,
        "/MAXTCP:",
        &tcp,
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

    let _ = user;
    Ok(())
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

pub fn create_nic(settings: &Settings, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let output = settings::vpncmd(settings, VpncmdAuth::Elevated, &["NicCreate", name])?;
    if crate::softether::text_says_success(&output.stdout) {
        Ok(())
    } else {
        Err(crate::vpn::last_error(&output.stdout))
    }
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
