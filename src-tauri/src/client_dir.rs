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
    let usable = missing.is_empty();
    let problem = if missing.is_empty() {
        None
    } else if crate::i18n::is_en() {
        Some(format!("Missing {}", missing.join(", ")))
    } else {
        Some(format!("缺少 {}", missing.join("、")))
    };

    ClientCandidate {
        dir: path_display(dir),
        source,
        usable,
        problem,
        version: if usable { probe_version(dir) } else { None },
        writable: is_dir_writable(dir),
    }
}

fn is_dir_writable(dir: &Path) -> bool {
    // vpnclient 要在这个目录里写自己的配置与日志。
    // 装到 /usr/local/vpnclient 的普通用户跑起来会因为写不进去而连不上，
    // 这个信息值得提前告诉用户。
    std::fs::metadata(dir).map(|m| !m.permissions().readonly()).unwrap_or(false)
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

/// 列出所有候选：内置 + 扫到的 + 用户指定的。
///
/// 顺序即优先级：内置在前（版本确定），扫到的按发现顺序，自定义单独标记。
/// 同一路径只出现一次——用户指定的目录恰好也是扫到的那个时，以后者身份展示。
pub fn candidates(custom_dir: Option<&str>) -> Vec<ClientCandidate> {
    let mut out: Vec<ClientCandidate> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    let bundled = PathBuf::from(BUNDLED_CLIENT_DIR);
    if bundled.is_dir() {
        let candidate = inspect(&bundled, ClientSource::Bundled);
        seen.push(candidate.dir.clone());
        out.push(candidate);
    }

    let mut roots = scan_roots_for_user();
    roots.extend(scan_package_cache());
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
            let dir = PathBuf::from(BUNDLED_CLIENT_DIR);
            if looks_like_client(&dir) {
                Ok(Resolved { dir, source, stale_reason: None })
            } else if dir.is_dir() {
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
}
