use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;

use softether_tray_lib::accounts::{self, AccountInput};
use softether_tray_lib::client_dir::{self, ClientCandidate};
use softether_tray_lib::settings::{self, Settings};
use softether_tray_lib::softether::{AccountDetail, DiagResult, NicInfo};
use softether_tray_lib::vpn::{self, Overview};

struct AppState {
    settings: Mutex<Settings>,
    /// 上次建托盘菜单时的内容签名。内容没变就不重建菜单——
    /// Linux 上 set_menu 是一趟 DBus 往返，2 秒一轮的轮询不该反复折腾托盘。
    tray_signature: Mutex<Option<String>>,
}

static TERMINATION_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_STARTED: AtomicBool = AtomicBool::new(false);

extern "C" fn termination_signal_handler(_: libc::c_int) {
    TERMINATION_REQUESTED.store(true, Ordering::SeqCst);
}

fn state_settings(state: &State<AppState>) -> Settings {
    state.settings.lock().map(|s| s.clone()).unwrap_or_default()
}

fn store_settings(state: &State<AppState>, settings: Settings) -> Result<(), String> {
    // 进内存前先归一化：界面拿到的必须是自洽的一份（风格/语言/客户端来源），
    // 否则写进来的错位值会一直显示到下次重启。
    let settings = settings.normalized();
    settings.save()?;
    *state.settings.lock().map_err(|_| {
        softether_tray_lib::i18n::tr("设置锁被占用", "The settings lock is busy").to_string()
    })? = settings;
    Ok(())
}

fn notify(app: &AppHandle, settings: &Settings, title: &str, body: &str) {
    if !settings.notifications {
        return;
    }
    // 通知失败不能影响连接动作本身，所以只记不抛。
    #[cfg(target_os = "linux")]
    let _ = app;
    #[cfg(target_os = "linux")]
    let result = notify_rust::Notification::new()
        .appname("SoftEther Tray")
        .summary(title)
        .body(body)
        .show();
    #[cfg(not(target_os = "linux"))]
    let result = app.notification().builder().title(title).body(body).show();
    if let Err(error) = result {
        let message = if softether_tray_lib::i18n::is_en() {
            format!("Failed to show the desktop notification (the connection itself completed): {error}")
        } else {
            format!("桌面通知发送失败（连接本身已完成）: {error}")
        };
        let _ = app.emit("action-error", message);
    }
}

fn rebuild_tray(app: &AppHandle, overview: &Overview) -> tauri::Result<()> {
    let handle = app.clone();
    let connected = overview.snapshot.as_ref().filter(|s| s.connected);

    let tooltip = connected
        .map(|snapshot| {
            if softether_tray_lib::i18n::is_en() {
                format!("SoftEther VPN · {} connected", snapshot.name)
            } else {
                format!("SoftEther VPN · {} 已连接", snapshot.name)
            }
        })
        .unwrap_or_else(|| {
            if overview.service_running {
                softether_tray_lib::i18n::tr("SoftEther VPN · 未连接", "SoftEther VPN · Disconnected")
                    .to_string()
            } else {
                softether_tray_lib::i18n::tr("SoftEther VPN · 服务未启动", "SoftEther VPN · Service stopped")
                    .to_string()
            }
        });

    // 菜单项先算成 (id, label) 列表：签名要拿它们比对，没变就整轮跳过重建
    let mut account_items: Vec<(String, String)> = Vec::new();
    if overview.accounts.is_empty() {
        // 空清单有两种成因，别混成一句：客户端没跑时 accounts 必然为空，
        // 这时显示「还没有连接配置」是误导，该直接给一个启动服务的入口。
        if !overview.service_running {
            account_items.push((
                "start-service".into(),
                softether_tray_lib::i18n::tr("（服务未启动）点击启动服务", "(Service stopped) Click to start")
                    .into(),
            ));
        } else {
            account_items.push((
                "no-account".into(),
                softether_tray_lib::i18n::tr("（还没有连接配置）", "(No connection profiles)").into(),
            ));
        }
    }
    let mut seen = HashSet::new();
    for account in &overview.accounts {
        if !seen.insert(account.name.clone()) {
            continue;
        }
        let is_up = softether_tray_lib::softether::status_is_connected(&account.status);
        let label = if is_up {
            if softether_tray_lib::i18n::is_en() {
                format!("Disconnect {}", account.name)
            } else {
                format!("断开 {}", account.name)
            }
        } else if softether_tray_lib::i18n::is_en() {
            format!("Connect {}", account.name)
        } else {
            format!("连接 {}", account.name)
        };
        let id = if is_up { format!("disconnect:{}", account.name) } else { format!("connect:{}", account.name) };
        account_items.push((id, label));
    }

    // tooltip、账户项就是菜单的全部可变内容；语言变化也必须触发重建
    let signature = format!(
        "{}\n{tooltip}\n{}",
        softether_tray_lib::i18n::current(),
        account_items.iter().map(|(id, label)| format!("{id}={label}")).collect::<Vec<_>>().join("\n")
    );
    let unchanged = app
        .state::<AppState>()
        .tray_signature
        .lock()
        .map(|last| last.as_deref() == Some(signature.as_str()))
        .unwrap_or(false);
    if unchanged && handle.tray_by_id("main").is_some() {
        return Ok(());
    }

    let open = MenuItem::with_id(
        &handle,
        "open",
        softether_tray_lib::i18n::tr("打开 SoftEther Tray", "Open SoftEther Tray"),
        true,
        None::<&str>,
    )?;
    let separator = || PredefinedMenuItem::separator(&handle);
    let diag = MenuItem::with_id(
        &handle,
        "diag",
        softether_tray_lib::i18n::tr("网络诊断…", "Network diagnostics…"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        &handle,
        "quit",
        softether_tray_lib::i18n::tr("退出 SoftEther Tray", "Quit SoftEther Tray"),
        true,
        None::<&str>,
    )?;

    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> = Vec::new();
    items.push(Box::new(open));
    items.push(Box::new(separator()?));
    for (id, label) in account_items {
        items.push(Box::new(MenuItem::with_id(&handle, id, label, true, None::<&str>)?));
    }
    items.push(Box::new(separator()?));
    items.push(Box::new(diag));
    items.push(Box::new(quit));
    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = items.iter().map(|item| item.as_ref()).collect();
    let menu = Menu::with_items(&handle, &refs)?;

    if let Some(tray) = handle.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
        tray.set_tooltip(Some(&tooltip))?;
    } else {
        let mut builder = TrayIconBuilder::with_id("main")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .tooltip(&tooltip)
            .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()))
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    show_window(tray.app_handle());
                }
            });
        if let Some(icon) = handle.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(&handle)?;
    }

    // 只有真建成功才记签名，失败的下一次调用要能重试
    if let Ok(mut last) = app.state::<AppState>().tray_signature.lock() {
        *last = Some(signature);
    }
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        // 最小化的窗口不能只靠 show/unminimize/set_focus：tao 的 set_focus 在窗口
        // 最小化时会把焦点请求直接丢弃，而它恢复用的 deiconify 底层只是 XMapWindow，
        // DDE 这类窗口管理器根本不认，窗口原地不动。hide→show 先向 WM 撤回再重新
        // 映射，重新映射的窗口必然按 Normal 状态显示，焦点请求随后才发得出去。
        let minimized = window.is_minimized().unwrap_or(false);
        if minimized {
            let _ = window.hide();
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if minimized {
            // 「已最小化」标志要等窗口状态事件才清掉，那之前 set_focus 依旧会被
            // tao 丢弃，所以稍后补一次；标志没清掉说明窗口没被重映射成功，再走一遍。
            let window = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(150));
                if window.is_minimized().unwrap_or(false) {
                    let _ = window.hide();
                    let _ = window.show();
                }
                let _ = window.set_focus();
            });
        }
    }
}

/// dev 场景兜底：裸跑 debug 二进制（没起 vite）时 webview 只能加载到浏览器的连接
/// 失败页，而窗口是无边框的——页内标题栏跟着整页一起丢，窗口既拖不动也关不掉。
/// 探到 dev_url 没人在听就换回系统标题栏，再把报错页换成说人话的提示。
/// 正式版前端内嵌、无 dev_url，永远走不到这里；vite 在跑时探测成功，什么都不做。
fn dev_server_fallback(handle: &AppHandle, window: &tauri::WebviewWindow) {
    if !tauri::is_dev() {
        return;
    }
    let Some(dev_url) = handle.config().build.dev_url.clone() else {
        return;
    };
    // 只认本机回环地址：远程 dev_url 是否在跑无从判断，别乱动窗口
    match dev_url.host_str() {
        Some("localhost" | "127.0.0.1" | "::1") => {}
        _ => return,
    }
    let Some(port) = dev_url.port_or_known_default() else {
        return;
    };
    // 探测放后台线程：连不上要等超时，不能拖住启动
    let window = window.clone();
    std::thread::spawn(move || {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
        use std::time::Duration;
        // localhost 可能解析成 IPv4 或 IPv6，任一通就认为 vite 在跑
        let reachable = [IpAddr::from(Ipv4Addr::LOCALHOST), IpAddr::from(Ipv6Addr::LOCALHOST)]
            .into_iter()
            .any(|ip| {
                TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_millis(400))
                    .is_ok()
            });
        if reachable {
            return;
        }
        let _ = window.set_decorations(true);
        let _ = window.navigate(fallback_page_url());
    });
}

/// 前端没加载起来时的兜底页。纯 data: URL、不碰 IPC：窗口操作全交给系统标题栏。
fn fallback_page_url() -> tauri::Url {
    let tr = softether_tray_lib::i18n::tr;
    let html = format!(
        r##"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>SoftEther Tray</title>
<style>
  body {{ margin: 0; min-height: 100vh; display: flex; align-items: center; justify-content: center; background: #f3f3f3; color: #1f1f1f; font: 13px/1.7 "Segoe UI Variable Text", "Segoe UI", "Noto Sans CJK SC", "Source Han Sans SC", system-ui, sans-serif; }}
  .card {{ width: 430px; box-sizing: border-box; background: #fff; border: 1px solid #e5e5e5; border-radius: 8px; padding: 28px 32px; }}
  .mark {{ width: 32px; height: 32px; display: block; margin-bottom: 14px; }}
  h1 {{ font-size: 15px; font-weight: 600; margin: 0 0 10px; }}
  p {{ margin: 0 0 10px; color: #5c5c5c; }}
  code {{ background: #fafafa; border: 1px solid #e5e5e5; border-radius: 4px; padding: 1px 6px; font-size: 12px; color: #1f1f1f; }}
  .hint {{ color: #8a8a8a; font-size: 12px; margin: 0; }}
</style>
</head>
<body>
  <div class="card">
    <svg class="mark" viewBox="0 0 256 256">
      <rect width="256" height="256" rx="58" fill="#2a2a2a" />
      <path d="M70 96a58 58 0 0 1 116 0" fill="none" stroke="#fff" stroke-width="14" stroke-linecap="round" />
      <circle cx="128" cy="140" r="26" fill="none" stroke="#fff" stroke-width="14" />
      <path d="M104 188h48" stroke="#fff" stroke-width="14" stroke-linecap="round" />
      <circle cx="128" cy="58" r="9" fill="#5dcaa5" />
    </svg>
    <h1>{title}</h1>
    <p>{desc}</p>
    <p>{howto}</p>
    <p class="hint">{hint}</p>
  </div>
</body>
</html>"##,
        title = tr("界面未能加载", "The interface could not be loaded"),
        desc = tr(
            "没有检测到前端开发服务器（localhost:1420），应用界面加载失败。",
            "The frontend dev server (localhost:1420) is not running, so the interface could not load.",
        ),
        howto = tr(
            "请在项目目录运行 <code>npm run tauri dev</code>（或先运行 <code>npm run dev</code>），然后重新打开应用。",
            "Run <code>npm run tauri dev</code> (or <code>npm run dev</code> first) in the project directory, then reopen the app.",
        ),
        hint = tr(
            "窗口已改用系统标题栏，可正常拖动与关闭。",
            "The window now uses the system title bar, so it can still be moved and closed.",
        ),
    );
    // HTML 整体百分号编码：url crate 会把 data: 当不透明路径处理，裸留空格、引号
    // 会被二次改写，编码到只剩字母数字最稳。
    let mut url = String::with_capacity(html.len() * 3 + 64);
    url.push_str("data:text/html;charset=utf-8,");
    for byte in html.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => url.push(byte as char),
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    tauri::Url::parse(&url).expect("generated data URL must parse")
}

/// 退出前的统一收尾：断开当前 VPN；是否停止客户端服务由设置决定。
///
/// 托盘菜单的「退出」和界面里的「退出应用」都必须走这里，
/// 否则从托盘退出会绕过清理，留下仍在跑的客户端。
fn shutdown_client(app: &AppHandle) {
    if SHUTDOWN_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let settings = state_settings(&app.state::<AppState>());
    // 应用退出总是断开当前账户，但不擅自关闭用户自己启动的客户端服务。
    if vpn::service_running() {
        let overview = vpn::read_overview_fresh(&settings);
        if let Some(name) = vpn::active_account(&settings, &overview.accounts) {
            let _ = do_disconnect(app, &name);
        }
    }
    // 只有应用自己启动的客户端，且用户打开了对应设置，才停止服务。
    if settings.stop_client_on_exit && vpn::managed_by_app() && vpn::service_running() {
        // 退出优先：最多等 20 秒授权，超时就带着仍在跑的客户端退出
        let _ = vpn::stop_service(&settings, Some(20));
        vpn::clear_managed();
    }
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "quit" => {
            shutdown_client(app);
            app.exit(0);
        }
        "open" => show_window(app),
        "start-service" => start_service_from_tray(app),
        "diag" => {
            show_window(app);
            let _ = app.emit("open-diagnostics", ());
        }
        other => {
            if let Some(name) = other.strip_prefix("connect:") {
                connect_account(app, name.to_string());
            } else if let Some(name) = other.strip_prefix("disconnect:") {
                disconnect_account(app, name.to_string());
            }
        }
    }
}

/// 轮询路径：允许用短缓存，避免每 2 秒为一堆不变的数据反复冷启动 vpncmd。
fn current_overview(app: &AppHandle) -> Overview {
    let settings = state_settings(&app.state::<AppState>());
    vpn::read_overview(&settings)
}

/// 动作路径：忽略缓存。连接/断开之后必须看到真实结果，不能把动作之前的
/// 快照当成结果回填给界面。
fn fresh_overview(app: &AppHandle) -> Overview {
    let settings = state_settings(&app.state::<AppState>());
    vpn::read_overview_fresh(&settings)
}

/// 把一份现成的 Overview 同步到托盘与前端。
fn publish_with(app: &AppHandle, overview: &Overview) {
    let _ = rebuild_tray(app, overview);
    let _ = app.emit("overview", overview);
}

/// 读一份最新状态并广播出去，把同一份结果交给调用方复用。
/// 一次 read_overview_fresh 要串行跑好几个子进程（vpncmd ×3、ip、pgrep），
/// 谁都不许再单独读第二遍——那会在同一次操作里把整套开销翻倍。
fn publish(app: &AppHandle) -> Overview {
    let overview = fresh_overview(app);
    publish_with(app, &overview);
    overview
}

fn report_error(app: &AppHandle, title: &str, error: &str) {
    let settings = state_settings(&app.state::<AppState>());
    notify(app, &settings, title, error);
    let _ = app.emit("action-error", error.to_string());
}

fn notify_connected(app: &AppHandle, name: &str, overview: &Overview) {
    let settings = state_settings(&app.state::<AppState>());
    let body = overview.iface_ipv4.clone().unwrap_or_else(|| {
        softether_tray_lib::i18n::tr("已建立隧道", "Tunnel established").to_string()
    });
    notify(
        app,
        &settings,
        softether_tray_lib::i18n::tr("VPN 已连接", "VPN connected"),
        &format!("{name} · {body}"),
    );
}

fn notify_disconnected(app: &AppHandle, name: &str) {
    let settings = state_settings(&app.state::<AppState>());
    notify(
        app,
        &settings,
        softether_tray_lib::i18n::tr("VPN 已断开", "VPN disconnected"),
        &format!(
            "{name} · {}",
            softether_tray_lib::i18n::tr("已清理网段路由", "Subnet routes cleaned up")
        ),
    );
}

/// 托盘里的「启动服务」：只拉起客户端，不替用户自动连某个账户——
/// 该连哪个由菜单里点谁决定，这里不能猜。
fn start_service_from_tray(app: &AppHandle) {
    let settings = state_settings(&app.state::<AppState>());
    match vpn::start_service(&settings) {
        Ok(spawned) => {
            if spawned {
                vpn::mark_managed();
            }
            // 服务刚起来，账户清单还是空的，必须重新读一份再广播
            publish(app);
        }
        Err(error) => {
            report_error(
                app,
                softether_tray_lib::i18n::tr("客户端未启动", "Service did not start"),
                &error,
            );
            publish(app);
        }
    }
}

fn connect_account(app: &AppHandle, name: String) {
    match do_connect(app, &name) {
        Ok(overview) => {
            notify_connected(app, &name, &overview);
            publish_with(app, &overview);
        }
        Err(error) => {
            report_error(app, softether_tray_lib::i18n::tr("连接失败", "Connection failed"), &error);
            // 失败时状态可能已经半变，重新读一份再广播
            publish(app);
        }
    }
}

fn disconnect_account(app: &AppHandle, name: String) {
    match do_disconnect(app, &name) {
        Ok(overview) => {
            notify_disconnected(app, &name);
            publish_with(app, &overview);
        }
        Err(error) => {
            report_error(
                app,
                softether_tray_lib::i18n::tr("断开失败", "Disconnection failed"),
                &error,
            );
            publish(app);
        }
    }
}

fn do_connect(app: &AppHandle, name: &str) -> Result<Overview, String> {
    let settings = state_settings(&app.state::<AppState>());
    // 只有本次真把客户端拉起来才接管归属；用户自己开着的客户端不算我们的
    if vpn::start_service(&settings)? {
        vpn::mark_managed();
    }
    let current = vpn::account_snapshot(&settings, name);
    let already = current.as_ref().map(|snap| snap.connected).unwrap_or(false);
    let stuck = current
        .as_ref()
        .map(|snap| {
            !snap.connected && softether_tray_lib::softether::status_is_connecting(&snap.state)
        })
        .unwrap_or(false);

    // 半连接（客户端在无限重连）要先收掉：「连接中」的账户再发 AccountConnect
    // 会直接被客户端拒掉——错误代码 35「指定的 VPN 连接设置当前已连接」。
    // 用户点了连接就是想要一次新的尝试，所以断开重来，而不是干等它自己重连。
    if stuck {
        let _ = settings::vpncmd(&settings, settings::VpncmdAuth::Plain, &["AccountDisconnect", name]);
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    // 并发点击和自动连接可能同时进来；真实连接已经存在时不再发第二条 AccountConnect。
    if !already {
        let output = settings::vpncmd(&settings, settings::VpncmdAuth::Plain, &["AccountConnect", name])?;
        if !softether_tray_lib::softether::text_says_success(&output.stdout) {
            return Err(vpn::last_error(&output.stdout));
        }
    }

    // 会话建立是异步的：状态真变已连接之后才动路由，避免往死网卡塞路由。
    //
    // 30 秒不是随手写的：远程服务器要 TLS 握手 + 用户认证，实测正常连接
    // 就要十几秒（会话状态走「开始连接 → 重试 → 验证用户身份」）。原来只给
    // 8 秒，慢一点的服务器会被误判成超时，而"超时"和"真连不上"在界面上
    // 长得一模一样，用户根本分不清是网络慢还是配置错。
    //
    // 等待期间记下最后一次会话状态，报错时才能说清卡在"重试"还是压根没开始。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut last_state = String::new();
    let mut connected = already;
    while !connected && std::time::Instant::now() < deadline {
        match vpn::account_snapshot(&settings, name) {
            Some(snap) if snap.connected => {
                connected = true;
                break;
            }
            Some(snap) => last_state = snap.state,
            None => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if !connected {
        // 客户端还在后台无限重连：不收掉的话界面会一直挂着「连接中」，
        // 而且它会持续占着会话反复重试。
        let _ = settings::vpncmd(&settings, settings::VpncmdAuth::Plain, &["AccountDisconnect", name]);
        return Err(connect_timeout_error(name, &last_state));
    }

    // 隧道通了但网卡没地址是 Linux 客户端的常态：它只管链路层，IP 要自己弄。
    // DHCP、推导网关、写路由三件事合并在**一次提权**里，用户只输一遍密码。
    //
    // 两条路，优先第一条：
    //   1. 装了 polkit 授权 → `pkexec <helper>`，命中规则时**完全静默**；
    //   2. 没装 → 退回 `pkexec sh -c <内联脚本>`，每次弹一次密码框。
    // 两条路的语义完全一致，helper 只是把同一套 shell 逻辑固化进一个受控入口。
    let overview = vpn::read_overview_fresh(&settings);
    if let Some(iface) = overview.iface.clone() {
        let need_lease = overview.iface_ipv4.is_none();
        let output = if settings::privileged_helper().is_some() {
            let nets = settings.routes.subnets.join(",");
            let mtu = settings.routes.mtu.to_string();
            let mut args = vec!["connect", iface.as_str(), mtu.as_str(), nets.as_str()];
            if need_lease {
                args.push("--lease");
            }
            settings::run_helper(&args).map_err(|e| {
                decorate_net_error(e, &iface, need_lease)
            })?
        } else {
            let script = settings::connect_routes_script(&iface, &settings.routes, need_lease)?;
            settings::run_privileged(&script).map_err(|e| {
                decorate_net_error(e, &iface, need_lease)
            })?
        };
        // 提权跑完再看一眼真实状态，别拿脚本的回显当结论
        let after = vpn::read_overview_fresh(&settings);
        if after.iface_ipv4.is_none() {
            let hint = if settings::has_dhcp_client() {
                if softether_tray_lib::i18n::is_en() {
                    format!(
                        "Virtual adapter {iface} still has no IPv4 address (DHCP did not respond). \
                         Enable the Virtual DHCP Server on the server HUB, or assign a static address and retry."
                    )
                } else {
                    format!(
                        "虚拟网卡 {iface} 仍未拿到 IPv4（DHCP 没有响应）。请在服务端 HUB 上启用 \
                         「虚拟 DHCP 服务器」，或手动配置静态地址后重试"
                    )
                }
            } else if softether_tray_lib::i18n::is_en() {
                format!(
                    "Virtual adapter {iface} has no IPv4 address and no dhclient/dhcpcd is available. \
                     Install a DHCP client (e.g. sudo apt install isc-dhcp-client) or assign a static address to {iface}."
                )
            } else {
                format!(
                    "虚拟网卡 {iface} 未拿到 IPv4，且系统没有 dhclient/dhcpcd 可用。\
                     请先安装 DHCP 客户端（如 sudo apt install isc-dhcp-client），\
                     或手动为 {iface} 配置静态地址"
                )
            };
            return Err(hint);
        }
        if output.contains("NO_GATEWAY") {
            return Err(if softether_tray_lib::i18n::is_en() {
                format!("Cannot determine the tunnel gateway for {iface}; routes were not installed.")
            } else {
                format!("无法确定 {iface} 的隧道网关，路由未下发")
            });
        }
    }
    Ok(vpn::read_overview_fresh(&settings))
}

/// 等待隧道超时时能说的实话。
///
/// 光说"超时"没用：客户端可能卡在「重试」（多半是密码 / 用户 / HUB 不对，
/// 或服务端限流），也可能压根没开始握手。把最后看到的会话状态和对应的
/// 排查方向一起给出来，用户才知道该改配置还是该查网络。
fn connect_timeout_error(name: &str, last_state: &str) -> String {
    let state = last_state.trim();
    if softether_tray_lib::i18n::is_en() {
        if state.is_empty() {
            format!(
                "Timed out waiting for \"{name}\" to connect, and the client reported no session state. \
                 Check the client log and whether the server is reachable."
            )
        } else {
            format!(
                "Timed out waiting for \"{name}\" to connect: the client is stuck at \"{state}\" and keeps retrying. \
                 That usually means a wrong password, an unknown user or HUB name, or a server-side limit — \
                 check the profile and the client log."
            )
        }
    } else if state.is_empty() {
        format!(
            "等待「{name}」建立隧道超时，客户端也没有给出会话状态。\
             请查看客户端日志，并确认服务器地址可达"
        )
    } else {
        format!(
            "等待「{name}」建立隧道超时：客户端一直卡在「{state}」并反复重试。\
             多半是密码不对、用户名或 HUB 名称写错，或者服务端限制了这个账号——\
             请核对配置并查看客户端日志"
        )
    }
}

/// 把网卡配置那一步的失败翻译成人能看懂的话。
///
/// 两种失败都藏在 stderr 的标记里，必须先认出来再谈别的：
///   - `NO_ADDRESS`：网卡没有 IPv4，网关必然推不出来——该查 DHCP / 静态地址；
///   - `NO_GATEWAY`：有地址但推不出第一跳——该查路由表。
/// 反过来的话（把没地址报成 NO_GATEWAY）会把人引去翻路由，方向上就错了。
fn decorate_net_error(message: String, iface: &str, need_lease: bool) -> String {
    if message.contains("NO_ADDRESS") {
        return if softether_tray_lib::i18n::is_en() {
            format!(
                "Virtual adapter {iface} has no IPv4 address, so no tunnel gateway could be derived \
                 and routes were not installed. DHCP did not deliver a lease: enable the Virtual \
                 DHCP Server on the server HUB, or assign a static address to {iface}."
            )
        } else {
            format!(
                "虚拟网卡 {iface} 没有 IPv4 地址，推不出隧道网关，路由未下发。\
                 DHCP 没要来地址：请在服务端 HUB 上启用「虚拟 DHCP 服务器」，\
                 或手动为 {iface} 配置静态地址"
            )
        };
    }
    if message.contains("NO_GATEWAY") {
        return if softether_tray_lib::i18n::is_en() {
            format!(
                "Cannot determine the tunnel gateway for {iface}, so routes were not installed. \
                 Check that the adapter has an address and a route on it (`ip -4 route show dev {iface}`)."
            )
        } else {
            format!(
                "无法确定 {iface} 的隧道网关，路由未下发。\
                 请确认网卡上有地址和路由（`ip -4 route show dev {iface}`）"
            )
        };
    }
    decorate_dhcp_error(message, iface, need_lease)
}

/// helper 报 NoDhcpClient 或连接时缺 DHCP 客户端，都该给出可操作的安装提示。
fn decorate_dhcp_error(message: String, iface: &str, need_lease: bool) -> String {
    if need_lease && !settings::has_dhcp_client() {
        if softether_tray_lib::i18n::is_en() {
            format!(
                "{message}\nNo dhclient/dhcpcd is available, so {iface} cannot get an IP. \
                 Install a DHCP client (sudo apt install isc-dhcp-client) or assign a static address to {iface}."
            )
        } else {
            format!(
                "{message}\n系统没有 dhclient/dhcpcd，{iface} 拿不到 IP。\
                 请安装 DHCP 客户端（sudo apt install isc-dhcp-client），或手动为 {iface} 配静态地址"
            )
        }
    } else {
        message
    }
}

/// SoftEther 的 Linux 客户端不替我们改路由表。原来网关是在 Rust 里算好再拼进脚本的，
/// 现在推导挪进了 `connect_routes_script` 的 shell 里——只有那样才能和 DHCP 共用
/// 同一次 pkexec，否则用户连接一次要输两遍密码。

fn do_disconnect(app: &AppHandle, name: &str) -> Result<Overview, String> {
    let settings = state_settings(&app.state::<AppState>());
    let output = settings::vpncmd(&settings, settings::VpncmdAuth::Plain, &["AccountDisconnect", name])?;
    let acceptable = softether_tray_lib::softether::disconnect_acceptable(&output.stdout);
    if !acceptable {
        return Err(vpn::last_error(&output.stdout));
    }
    // 隧道已经断了，后面的清理是"打扫战场"，不该让它的失败否掉断开本身。
    // 只有当网卡上真有残留路由或 IP 时才提权——否则每次断开都弹一次密码框，
    // 用户会以为"断开连接"这个动作需要管理员权限，其实不需要。
    //
    // 注意这里读的是断开**之后**的状态：断之前网卡上必然有路由和地址，
    // 拿那个去判断等于永远要提权。
    if let Some(iface) = vpn::read_overview_fresh(&settings).iface {
        if settings::needs_disconnect_cleanup(&iface, settings.routes.dhcp) {
            // 与连接一致：装了 helper 就走 helper（静默），否则退回内联脚本
            let result = if settings::privileged_helper().is_some() {
                let nets = settings.routes.subnets.join(",");
                let mut args = vec!["cleanup", iface.as_str(), nets.as_str()];
                if settings.routes.dhcp {
                    args.push("--flush");
                }
                settings::run_helper(&args)
            } else {
                settings::disconnect_routes_script(
                    &iface,
                    &settings.routes.subnets,
                    settings.routes.dhcp,
                )
                .and_then(|script| settings::run_privileged(&script))
            };
            if let Err(err) = result {
                // 清理失败只提示，不改变"已断开"这个结论
                eprintln!(
                    "{}",
                    if softether_tray_lib::i18n::is_en() {
                        format!("Post-disconnect route cleanup did not finish: {err}")
                    } else {
                        format!("断开后的路由清理未完成: {err}")
                    }
                );
            }
        }
    }
    Ok(vpn::read_overview_fresh(&settings))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvInfo {
    autostart: bool,
    config_dir: String,
}

#[tauri::command]
async fn get_overview(app: AppHandle) -> Result<Overview, String> {
    // 首屏要准：宁可付一次完整读取的代价，也不能让用户先看到 30 秒前的账号清单
    Ok(fresh_overview(&app))
}

#[tauri::command]
async fn connect(app: AppHandle, name: String) -> Result<Overview, String> {
    // do_connect 里要等隧道建立（最长 30 秒）、还要等 polkit 授权，都是阻塞调用。
    // 直接放在 async 命令里会占死一个 runtime worker，界面轮询跟着一起卡住。
    let result = do_connect_blocking(&app, &name).await;
    // do_connect 成功时已带回最终状态，直接广播它；失败才重新读一份
    match &result {
        Ok(overview) => {
            notify_connected(&app, &name, overview);
            publish_with(&app, overview);
        }
        Err(_) => {
            publish(&app);
        }
    }
    result
}

/// 把 `do_connect` 丢到阻塞线程池里跑，别占着 async runtime 的 worker。
async fn do_connect_blocking(app: &AppHandle, name: &str) -> Result<Overview, String> {
    let task = {
        let app = app.clone();
        let name = name.to_string();
        tauri::async_runtime::spawn_blocking(move || do_connect(&app, &name))
    };
    match task.await {
        Ok(result) => result,
        Err(e) => Err(softether_tray_lib::i18n::tr(
            &format!("连接任务失败: {e}"),
            &format!("The connection task failed: {e}"),
        )
        .to_string()),
    }
}

#[tauri::command]
async fn disconnect(app: AppHandle, name: String) -> Result<Overview, String> {
    let result = do_disconnect(&app, &name);
    match &result {
        Ok(overview) => {
            notify_disconnected(&app, &name);
            publish_with(&app, overview);
        }
        Err(_) => {
            publish(&app);
        }
    }
    result
}

#[tauri::command]
async fn start_service(app: AppHandle) -> Result<Overview, String> {
    let settings = state_settings(&app.state::<AppState>());
    // 记下"客户端由本应用拉起"，退出时据此决定是否一并停止。
    // 客户端已在跑时不动标记：那是用户自己的，关掉应用不该连带关掉它。
    if vpn::start_service(&settings)? {
        vpn::mark_managed();
    }
    Ok(publish(&app))
}

/// 停止 SoftEther 客户端。会先断开当前账户并清路由，再停服务，避免残留黑洞路由。
#[tauri::command]
async fn stop_service(app: AppHandle) -> Result<Overview, String> {
    let settings = state_settings(&app.state::<AppState>());
    let overview = vpn::read_overview_fresh(&settings);
    if let Some(name) = vpn::active_account(&settings, &overview.accounts) {
        // 断开失败不阻断停机：客户端一停，隧道必然消失，后续清理由 stop_service 兜底
        let _ = do_disconnect(&app, &name);
    }
    vpn::stop_service(&settings, Some(120))?;
    vpn::clear_managed();
    Ok(publish(&app))
}

#[tauri::command]
fn client_managed() -> bool {
    vpn::managed_by_app()
}

/// 轮询状态。稳态下只读缓存 + sysfs（零子进程），只有缓存过期的那一轮才会
/// 冷启动 vpncmd。即便如此也必须放阻塞线程池：同步命令在主线程内联执行，
/// 一旦碰上"这一轮要真读"的那一刻，界面就会整段冻住。
#[tauri::command]
async fn refresh(app: AppHandle, fresh: Option<bool>) -> Result<Overview, String> {
    // `fresh` = 用户主动点了刷新：丢掉三层缓存（服务探测 2s / 清单 30s /
    // 会话 5s）重新读一遍，代价是一次 vpncmd 冷启动（约 250ms），手动点一次付得起。
    // 轮询路径不传（None），继续吃缓存，不然每 2 秒就要 fork 两次 vpncmd。
    let fresh = fresh.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let overview = if fresh {
            fresh_overview(&app)
        } else {
            current_overview(&app)
        };
        let _ = rebuild_tray(&app, &overview);
        // 刻意不 emit "overview"：返回值就是前端的更新通道，
        // 两条路都推会让同一份数据每 2 秒渲染两遍
        overview
    })
    .await
    .map_err(|e| {
        if softether_tray_lib::i18n::is_en() {
            format!("Failed to refresh status: {e}")
        } else {
            format!("刷新状态失败: {e}")
        }
    })
}

#[tauri::command]
async fn get_settings(app: AppHandle) -> Settings {
    state_settings(&app.state::<AppState>())
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    for subnet in &settings.routes.subnets {
        settings::validate_subnet(subnet)?;
    }
    settings::validate_mtu(settings.routes.mtu)?;
    validate_client_choice(&settings)?;
    // 返回归一化后的那一份：前端据此判断"磁盘上到底是什么"，
    // 原样回显会让「自己指定填了内置目录」这类错位一直留到下次打开
    let normalized = settings.normalized();
    store_settings(&app.state::<AppState>(), normalized.clone())?;
    softether_tray_lib::i18n::set_locale(&normalized.locale);
    publish(&app);
    Ok(normalized)
}

/// 保存前校验「来源 + 目录」这一对是否自洽。
///
/// 刻意允许「目录当前不可用」通过校验中的一部分情况：用户可能在
/// 客户端还没装好时就把设置填了。但**来源与目录的组合必须说得通**，
/// 否则保存进去的就是一份自相矛盾的配置，之后连接报的错会莫名其妙。
fn validate_client_choice(settings: &Settings) -> Result<(), String> {
    let source = settings
        .client_source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(source) = source else { return Ok(()) };
    let parsed = softether_tray_lib::client_dir::ClientSource::parse(source).ok_or_else(|| {
        if softether_tray_lib::i18n::is_en() {
            format!("Unknown client source: {source}")
        } else {
            format!("未知的客户端来源：{source}")
        }
    })?;
    let dir = settings.vpn_dir.as_deref().map(str::trim).filter(|s| !s.is_empty());
    match parsed {
        // 内置不看路径，存了也用不上
        softether_tray_lib::client_dir::ClientSource::Bundled => Ok(()),
        // 选「扫到的」/「自己指定」却没地址 = 保存后连接必然报错，当场拦下来
        softether_tray_lib::client_dir::ClientSource::Detected => {
            if dir.is_none() {
                return Err(softether_tray_lib::i18n::tr(
                    "已选择「搜索到的目录」，请先挑一个具体目录",
                    "A detected directory is selected; choose a specific directory first.",
                )
                .into());
            }
            Ok(())
        }
        softether_tray_lib::client_dir::ClientSource::Custom => {
            if dir.is_none() {
                return Err(softether_tray_lib::i18n::tr(
                    "已选择「自己指定目录」，请填写路径",
                    "A custom directory is selected; enter a path.",
                )
                .into());
            }
            Ok(())
        }
    }
}

/// 把「用哪个客户端」写进设置。
///
/// 来源没给或认不出来时**按目录反推**，别一律记成「自己指定」：内置目录
/// 被记成"自己指定"，界面就和实际跑的那份对不上（真踩过：连接配置页
/// 切回来之后客户端页显示"自己指定"，可跑的明明是内置客户端）。
/// 内置不看路径，`vpn_dir` 不写，和前端保存时的归一化口径一致。
fn apply_client_choice(settings: &mut Settings, dir: &str, source: Option<&str>) {
    let parsed = source
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(softether_tray_lib::client_dir::ClientSource::parse)
        .unwrap_or_else(|| softether_tray_lib::client_dir::derive_source(dir));
    settings.client_source = Some(parsed.as_str().to_string());
    settings.vpn_dir = match parsed {
        softether_tray_lib::client_dir::ClientSource::Bundled => None,
        _ => Some(dir.to_string()),
    };
}

/// 一份客户端目录里有多少个连接账号，以及它是不是当前正在用的那份。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountSource {
    dir: String,
    accounts: usize,
    current: bool,
    /// 这个目录的来源（内置 / 扫到的 / 自己指定）。
    /// 前端要拿它回传给「切换客户端」，传错来源会把内置客户端记成自己指定。
    source: softether_tray_lib::client_dir::ClientSource,
}

/// 各客户端目录各自的账号数。
///
/// 客户端的连接配置是**跟着客户端目录走**的（`vpn_client.config`），
/// 内置客户端和用户自装的那份互不相通。用户切了来源之后看到"暂无连接配置"，
/// 如果没有这个接口，界面只能显示一句干巴巴的空状态，没人能猜到
/// 账号其实在另一个目录里。
#[tauri::command]
async fn account_sources(app: AppHandle) -> Result<Vec<AccountSource>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = state_settings(&app.state::<AppState>());
        let current = softether_tray_lib::client_dir::resolve(
            settings
                .client_source
                .as_deref()
                .and_then(softether_tray_lib::client_dir::ClientSource::parse),
            settings.vpn_dir.as_deref(),
        )
        .ok()
        .map(|resolved| softether_tray_lib::settings::path_display(&resolved.dir));

        softether_tray_lib::client_dir::candidates(settings.vpn_dir.as_deref())
            .into_iter()
            .map(|candidate| AccountSource {
                accounts: softether_tray_lib::client_dir::account_count(std::path::Path::new(
                    &candidate.dir,
                )),
                current: current.as_deref() == Some(candidate.dir.as_str()),
                source: candidate.source,
                dir: candidate.dir,
            })
            .collect()
    })
    .await
    .map_err(|e| {
        if softether_tray_lib::i18n::is_en() {
            format!("Failed to read the client list: {e}")
        } else {
            format!("读取客户端列表失败: {e}")
        }
    })
}

/// 把内置客户端的配置文件属主交还给当前用户（只尝试一次）。
///
/// 客户端以 root 运行，`vpn_client.config` 会被写成 root:root 0600；
/// 应用是普通用户，读不到 → "连接配置"页里内置那一组永远是空的。
/// 交给特权助手 chown 一次就能长期可读（实测客户端重启不会改回 root）。
///
/// 只做一次：失败也不重试，避免每次进页面都去碰 pkexec。
static CLAIM_TRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn ensure_config_readable(dir: &str) {
    let path = std::path::Path::new(dir).join("vpn_client.config");
    // 能读就什么都不做；文件不存在说明客户端还没跑过，也没得改
    if std::fs::read(&path).is_ok() || !path.exists() {
        return;
    }
    if CLAIM_TRIED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    // 等 5 秒就够：polkit 免密时是毫秒级；需要输密码时不能把列表卡死
    let _ = settings::run_helper_with_timeout(&["claim-config", dir], Some(5));
}

/// 一组配置：某个客户端目录 + 它里面的连接配置。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientAccountGroup {
    dir: String,
    /// 展示用短名（内置 / 目录名），前端还会补上完整路径
    label: String,
    /// 是不是正在用的那份客户端
    current: bool,
    /// 这个目录的来源（内置 / 扫到的 / 自己指定）。
    /// 前端「切换并连接」要原样回传，否则内置客户端会被记成自己指定。
    source: softether_tray_lib::client_dir::ClientSource,
    accounts: Vec<softether_tray_lib::client_dir::StaticAccount>,
}

/// 把这台机器上**所有**客户端目录里的配置都列出来。
///
/// 连接配置跟着客户端目录走，用户往往在多份客户端里各有一批。
/// 只展示当前那份等于把别的藏起来——用户会以为配置丢了。
#[tauri::command]
async fn client_account_groups(app: AppHandle) -> Result<Vec<ClientAccountGroup>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = state_settings(&app.state::<AppState>());
        let current = softether_tray_lib::client_dir::resolve(
            settings
                .client_source
                .as_deref()
                .and_then(softether_tray_lib::client_dir::ClientSource::parse),
            settings.vpn_dir.as_deref(),
        )
        .ok()
        .map(|resolved| softether_tray_lib::settings::path_display(&resolved.dir));

        softether_tray_lib::client_dir::candidates(settings.vpn_dir.as_deref())
            .into_iter()
            // 文件不全或目录不可写的客户端，配置读了也不可信
            .filter(|candidate| candidate.usable)
            .filter_map(|candidate| {
                // 读不到配置（典型：root 所有 0600）就先让助手把属主交还，
                // 修好之后再读，否则这一组会静默消失
                ensure_config_readable(&candidate.dir);
                let path = std::path::Path::new(&candidate.dir);
                let accounts = softether_tray_lib::client_dir::accounts_in(path);
                // 空目录不占位：列表里只出现真有配置的客户端
                if accounts.is_empty() {
                    return None;
                }
                let label = match candidate.source {
                    softether_tray_lib::client_dir::ClientSource::Bundled => {
                        softether_tray_lib::i18n::tr("内置客户端", "Built-in client").to_string()
                    }
                    _ => path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| candidate.dir.clone()),
                };
                Some(ClientAccountGroup {
                    current: current.as_deref() == Some(candidate.dir.as_str()),
                    source: candidate.source,
                    label,
                    dir: candidate.dir,
                    accounts,
                })
            })
            .collect()
    })
    .await
    .map_err(|e| {
        if softether_tray_lib::i18n::is_en() {
            format!("Failed to read client profiles: {e}")
        } else {
            format!("读取各客户端的连接配置失败: {e}")
        }
    })
}

/// 在**另一个**客户端目录里连接某个配置：切客户端 → 启动 → 连接，一次做完。
///
/// 必须原子：那份配置只存在于它自己的客户端目录里。光切不连，用户还得再点一次；
/// 光连不切，连的是当前客户端里同名的另一个配置，几乎必然失败。
#[tauri::command]
async fn connect_in_client(
    app: AppHandle,
    dir: String,
    name: String,
    source: Option<String>,
) -> Result<Overview, String> {
    let settings = state_settings(&app.state::<AppState>());
    if vpn::service_running() {
        vpn::stop_service(&settings, Some(60))?;
    }
    let mut next = settings;
    apply_client_choice(&mut next, &dir, source.as_deref());
    validate_client_choice(&next)?;
    store_settings(&app.state::<AppState>(), next)?;

    let result = do_connect_blocking(&app, &name).await;
    match &result {
        Ok(overview) => {
            notify_connected(&app, &name, overview);
            publish_with(&app, overview);
        }
        Err(_) => {
            publish(&app);
        }
    }
    result
}

/// 切到另一个客户端目录：**停旧的 → 写设置 → 起新的**，一次做完。
///
/// 必须是原子动作。只写设置不停服务，界面显示新目录、实际还在跑旧那份，
/// 用户会以为切换生效了；而让用户自己先停再切，中间任何一步漏掉也是同一个结果。
/// 前端负责在调用前确认（"会停止当前服务，是否继续"），这里只管把事做完。
#[tauri::command]
async fn switch_client_dir(
    app: AppHandle,
    dir: String,
    // 用户点的是哪一张来源卡片。不传时按目录反推（内置目录=builtin），
    // 传进来的值必须是三种来源之一——乱传/不传都一律记成"自己指定"的话，
    // 前端从别的客户端切回内置时就会把来源写错。
    source: Option<String>,
) -> Result<Overview, String> {
    let settings = state_settings(&app.state::<AppState>());
    let was_running = vpn::service_running();
    if was_running {
        vpn::stop_service(&settings, Some(60))?;
    }
    let mut next = settings;
    apply_client_choice(&mut next, &dir, source.as_deref());
    validate_client_choice(&next)?;
    store_settings(&app.state::<AppState>(), next)?;
    let next = state_settings(&app.state::<AppState>());
    if vpn::start_service(&next)? {
        vpn::mark_managed();
    }
    Ok(publish(&app))
}

/// 列出所有可用的客户端目录（内置 / 扫到的 / 用户指定的）。
///
/// 扫盘是 IO 密集的，所以放在 async + spawn_blocking 里，
/// 避免用户点开设置页时把 UI 线程卡住。
#[tauri::command]
async fn list_client_candidates(app: AppHandle) -> Result<Vec<ClientCandidate>, String> {
    let settings = state_settings(&app.state::<AppState>());
    let custom = settings.vpn_dir.clone();
    tauri::async_runtime::spawn_blocking(move || client_dir::candidates(custom.as_deref()))
        .await
        .map_err(|e| {
            if softether_tray_lib::i18n::is_en() {
                format!("Failed to scan client directories: {e}")
            } else {
                format!("扫描客户端目录失败: {e}")
            }
        })
}

/// 单独体检一个用户手填的目录，给「校验」按钮用。
///
/// 和 list_client_candidates 分开，是为了让用户可以只验那一个路径，
/// 不必等一遍扫盘。
#[tauri::command]
async fn validate_client_dir(dir: String) -> Result<ClientCandidate, String> {
    let trimmed = dir.trim().to_string();
    if trimmed.is_empty() {
        return Err(softether_tray_lib::i18n::tr(
            "请先填写目录路径",
            "Enter a directory path first.",
        )
        .into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let path = std::path::PathBuf::from(&trimmed);
        let mut candidate = client_dir::inspect(&path, client_dir::ClientSource::Custom);
        if !path.is_dir() {
            candidate.usable = false;
            candidate.problem = Some(
                softether_tray_lib::i18n::tr("目录不存在", "Directory does not exist").into(),
            );
            candidate.version = None;
        }
        candidate
    })
    .await
    .map_err(|e| {
        if softether_tray_lib::i18n::is_en() {
            format!("Failed to validate the directory: {e}")
        } else {
            format!("校验目录失败: {e}")
        }
    })
}

#[tauri::command]
async fn account_detail(app: AppHandle, name: String) -> Result<AccountDetail, String> {
    let settings = state_settings(&app.state::<AppState>());
    vpn::account_detail(&settings, &name)
}

#[tauri::command]
async fn save_account(app: AppHandle, original: Option<String>, input: AccountInput) -> Result<Overview, String> {
    let settings = state_settings(&app.state::<AppState>());
    accounts::save_account(original.as_deref(), &input, &settings)?;
    Ok(publish(&app))
}

#[tauri::command]
async fn delete_account(app: AppHandle, name: String) -> Result<Overview, String> {
    let state = app.state::<AppState>();
    let settings = state_settings(&state);
    if settings.active_account.as_deref() == Some(name.as_str()) {
        let mut updated = settings.clone();
        updated.active_account = None;
        store_settings(&state, updated)?;
    }
    accounts::delete_account(&settings, &name)?;
    Ok(publish(&app))
}

#[tauri::command]
async fn create_nic(app: AppHandle, name: String) -> Result<Vec<NicInfo>, String> {
    let settings = state_settings(&app.state::<AppState>());
    accounts::create_nic(&settings, &name)?;
    Ok(vpn::read_overview_fresh(&settings).nics)
}

/// 删除虚拟网卡。用它的连接配置会失去网卡，必须先让用户确认（前端负责）。
#[tauri::command]
async fn delete_nic(app: AppHandle, name: String) -> Result<Vec<NicInfo>, String> {
    let settings = state_settings(&app.state::<AppState>());
    accounts::delete_nic(&settings, &name)?;
    let overview = vpn::read_overview_fresh(&settings);
    let _ = rebuild_tray(&app, &overview);
    Ok(overview.nics)
}

/// 启用 / 禁用虚拟网卡。
#[tauri::command]
async fn set_nic_enabled(
    app: AppHandle,
    name: String,
    enabled: bool,
) -> Result<Vec<NicInfo>, String> {
    let settings = state_settings(&app.state::<AppState>());
    accounts::set_nic_enabled(&settings, &name, enabled)?;
    Ok(vpn::read_overview_fresh(&settings).nics)
}

#[tauri::command]
async fn run_diagnostics(target: String, count: u32) -> Result<DiagResult, String> {
    tauri::async_runtime::spawn_blocking(move || accounts::ping(&target, count))
        .await
        .map_err(|e| {
            if softether_tray_lib::i18n::is_en() {
                format!("Diagnostics task failed: {e}")
            } else {
                format!("诊断任务失败: {e}")
            }
        })?
}

/// 诊断页「没填目标」时用的默认地址：当前隧道网段的第一跳。
///
/// 由当前网络状态算出来，而不是把某个内网地址写死在配置里——每个人的内网都不一样，
/// 写死的地址在别人机器上要么不存在、要么是别人的服务。没有隧道时返回 null，
/// 前端就退回到让用户自己填。
#[tauri::command]
async fn diag_default_target(app: AppHandle) -> Result<Option<String>, String> {
    let overview = current_overview(&app);
    Ok(overview.iface.as_deref().and_then(settings::interface_gateway))
}

/// 单个网段的验证结果。
///
/// 三种状态对应三种不同的失败原因，用户看到的提示要能区分：
///   Ok      —— 路由生效且隧道那侧有响应
///   NoRoute —— 这个网段的流量根本没走 VPN（被物理网卡接走了，内网自然不通）
///   Dead    —— 路由对，但隧道那侧不通（服务端问题／DHCP 没给地址）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteCheck {
    subnet: String,
    state: String,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteReport {
    iface: String,
    checks: Vec<RouteCheck>,
    summary: String,
}

/// 验证「设置里配置的网段」是否真的经隧道可达。
///
/// 刻意**不做自动发现**：Linux 版客户端没有路由推送能力（Windows 版才有），
/// 内网网段本地问不出来。实测过两种猜法都不成立——猜 .1/.254 当网关
/// （10.0.0.1 不存在但 10.10.4.160 通），读 ARP 表（只看得到同广播域）。
/// 与其给个"看着自动其实漏网段"的假功能，不如可靠回答"我填的这些通不通"。
#[tauri::command]
async fn verify_routes(app: AppHandle) -> Result<RouteReport, String> {
    let settings = state_settings(&app.state::<AppState>());
    let overview = vpn::read_overview_fresh(&settings);
    let iface = overview.iface.clone().ok_or_else(|| {
        softether_tray_lib::i18n::tr(
            "没有找到虚拟网卡，请先连接 VPN",
            "No virtual adapter found. Connect the VPN first.",
        )
        .to_string()
    })?;
    if overview.iface_ipv4.is_none() {
        return Err(if softether_tray_lib::i18n::is_en() {
            format!("Virtual adapter {iface} has no IPv4 address yet. Connect the VPN first.")
        } else {
            format!("虚拟网卡 {iface} 还没有 IPv4 地址，请先连接 VPN")
        });
    }

    let subnets = settings.routes.subnets.clone();
    if subnets.is_empty() {
        return Err(softether_tray_lib::i18n::tr(
            "设置里没有配置任何网段，先添加要经 VPN 访问的内网网段",
            "No subnets are configured. Add the intranet subnets that should go through the VPN first.",
        )
        .into());
    }

    let nets_arg = subnets.join(",");
    let raw = if settings::privileged_helper().is_some() {
        tauri::async_runtime::spawn_blocking({
            let iface = iface.clone();
            let nets = nets_arg.clone();
            move || settings::run_helper(&["probe", iface.as_str(), nets.as_str()])
        })
        .await
        .map_err(|e| {
            if softether_tray_lib::i18n::is_en() {
                format!("Route verification task failed: {e}")
            } else {
                format!("验证任务失败: {e}")
            }
        })??
    } else {
        // 没装 helper 时也能用，只是会弹一次密码框。
        // 输出格式必须与 helper 的 probe 完全一致，否则解析层要写两套；
        // 网关推导也要与 connect 一致（默认路由的 via 优先），否则"验证"和
        // "下发"用的不是同一个网关，通不通的结论就没有意义。
        let script = format!(
            "iface={iface}\nnets={nets_arg}\n\
             gw=$(ip -4 route show default dev $iface 2>/dev/null | awk '/via/ {{for (i = 1; i <= NF; i++) if ($i == \"via\") {{print $(i + 1); exit}}}}')\n\
             [ -n \"$gw\" ] || gw=$(ip -4 route show dev $iface | awk '/proto kernel/ && / src / {{print $1; exit}}' | cut -d/ -f1 | awk -F. 'BEGIN{{OFS=\".\"}} NF==4{{$4=1; print}}')\n\
             [ -n \"$gw\" ] || {{ echo NO_GATEWAY >&2; exit 3; }}\n\
             if ping -c 1 -W 2 -I $iface $gw >/dev/null 2>&1; then echo 'TUNNEL up'; else echo 'TUNNEL down'; fi\n\
             IFS=,; set -- $nets\n\
             for net in \"$@\"; do\n\
             if ip -4 route get \"${{net%%/*}}\" 2>/dev/null | grep -q \"dev $iface\"; then echo \"OK $net\"; else echo \"NOROUTE $net\"; fi\n\
             done"
        );
        settings::run_privileged(&script)?
    };

    let mut checks = Vec::new();
    let mut tunnel_up = true;
    for line in raw.lines() {
        let line = line.trim();
        if line == "TUNNEL down" {
            tunnel_up = false;
        } else if line == "TUNNEL up" {
            tunnel_up = true;
        } else if let Some(rest) = line.strip_prefix("OK ") {
            checks.push(RouteCheck {
                subnet: rest.to_string(),
                state: "ok".into(),
                detail: softether_tray_lib::i18n::tr(
                    "路由生效，流量经 VPN 转发",
                    "Route is active; traffic is forwarded through the VPN",
                )
                .into(),
            });
        } else if let Some(rest) = line.strip_prefix("NOROUTE ") {
            checks.push(RouteCheck {
                subnet: rest.to_string(),
                state: "noRoute".into(),
                detail: softether_tray_lib::i18n::tr(
                    "流量没走 VPN，被物理网卡接走了",
                    "Traffic is not using the VPN and is leaving via the physical adapter",
                )
                .into(),
            });
        } else if let Some(rest) = line.strip_prefix("DEAD ") {
            checks.push(RouteCheck {
                subnet: rest.to_string(),
                state: "dead".into(),
                detail: softether_tray_lib::i18n::tr(
                    "路由在，但隧道那侧没有响应",
                    "The route exists, but the tunnel side does not respond",
                )
                .into(),
            });
        }
    }

    if checks.is_empty() {
        return Err(if softether_tray_lib::i18n::is_en() {
            format!("No verification results were returned: {}", raw.trim())
        } else {
            format!("没有拿到验证结果：{}", raw.trim())
        });
    }

    let ok = checks.iter().filter(|c| c.state == "ok").count();
    let bad = checks.len() - ok;
    let summary = if !tunnel_up {
        softether_tray_lib::i18n::tr(
            "隧道不通：网关无响应，先检查连接状态",
            "Tunnel is down: the gateway does not respond; check the connection first",
        )
        .to_string()
    } else if bad == 0 {
        if softether_tray_lib::i18n::is_en() {
            format!("Routes for all {ok} subnets are active")
        } else {
            format!("{ok} 个网段的路由全部生效")
        }
    } else if softether_tray_lib::i18n::is_en() {
        format!("{ok} subnet(s) OK, {bad} with problems")
    } else {
        format!("{ok} 个正常，{bad} 个有问题")
    };

    Ok(RouteReport { iface, checks, summary })
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| {
            if softether_tray_lib::i18n::is_en() {
                format!("Failed to enable launch at login: {e}")
            } else {
                format!("开启开机自启动失败: {e}")
            }
        })?;
    } else {
        manager.disable().map_err(|e| {
            if softether_tray_lib::i18n::is_en() {
                format!("Failed to disable launch at login: {e}")
            } else {
                format!("关闭开机自启动失败: {e}")
            }
        })?;
    }
    Ok(manager.is_enabled().unwrap_or(enabled))
}

#[tauri::command]
fn autostart_enabled(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn env_info(app: AppHandle) -> EnvInfo {
    use tauri_plugin_autostart::ManagerExt;
    EnvInfo {
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        config_dir: settings::config_dir().display().to_string(),
    }
}

#[tauri::command]
fn config_dir_path() -> String {
    settings::config_dir().display().to_string()
}

#[tauri::command]
fn open_softether_site() -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg("https://www.softether.org")
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn open_config_dir() -> Result<String, String> {
    let dir = settings::config_dir();
    // 配置目录只在第一次保存设置时才建，全新安装时 xdg-open 会打不开不存在的路径
    std::fs::create_dir_all(&dir).map_err(|e| {
        if softether_tray_lib::i18n::is_en() {
            format!("Cannot create config directory {}: {e}", dir.display())
        } else {
            format!("无法创建配置目录 {}: {e}", dir.display())
        }
    })?;
    std::process::Command::new("xdg-open")
        .arg(&dir)
        .spawn()
        .map_err(|e| {
            if softether_tray_lib::i18n::is_en() {
                format!("Cannot open {}: {e}", dir.display())
            } else {
                format!("无法打开 {}: {e}", dir.display())
            }
        })?;
    Ok(dir.display().to_string())
}

/// 打开任意一个已存在的目录（各客户端目录分组各自的那份，不是应用设置目录）。
#[tauri::command]
fn open_dir(path: String) -> Result<String, String> {
    let dir = std::path::Path::new(&path);
    // 前端传来的路径必须是已存在的目录，别让 xdg-open 拿到文件或空路径
    if !dir.is_dir() {
        return Err(if softether_tray_lib::i18n::is_en() {
            format!("Not a directory: {}", dir.display())
        } else {
            format!("不是有效的目录: {}", dir.display())
        });
    }
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map_err(|e| {
            if softether_tray_lib::i18n::is_en() {
                format!("Cannot open {}: {e}", dir.display())
            } else {
                format!("无法打开 {}: {e}", dir.display())
            }
        })?;
    Ok(dir.display().to_string())
}

#[tauri::command]
async fn quit_app(app: AppHandle) -> Result<(), String> {
    shutdown_client(&app);
    app.exit(0);
    Ok(())
}

fn main() {
    // 让托盘图标同时以「图标主题名」发布（见 vendor/tray-icon 的 patch）。
    // deb 把图标装在 /usr/share/icons/hicolor/*/apps/softether-tray.png，
    // 所以这个名字与打包产物自洽；DDE 只认 IconName，不给就不渲染图标。
    std::env::set_var("TRAY_ICON_NAME", "softether-tray");
    let initial = Settings::load();
    softether_tray_lib::i18n::set_locale(&initial.locale);
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(AppState {
            settings: Mutex::new(initial),
            tray_signature: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_overview,
            connect,
            disconnect,
            start_service,
            stop_service,
            client_managed,
            refresh,
            get_settings,
            save_settings,
            account_detail,
            save_account,
            delete_account,
            create_nic,
            delete_nic,
            set_nic_enabled,
            run_diagnostics,
            diag_default_target,
            verify_routes,
            set_autostart,
            autostart_enabled,
            env_info,
            config_dir_path,
            open_config_dir,
            open_dir,
            list_client_candidates,
            account_sources,
            client_account_groups,
            connect_in_client,
            switch_client_dir,
            validate_client_dir,
            open_softether_site,
            quit_app
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            #[cfg(target_os = "linux")]
            unsafe {
                libc::signal(
                    libc::SIGTERM,
                    termination_signal_handler as *const () as libc::sighandler_t,
                );
            }
            #[cfg(target_os = "linux")]
            {
                let signal_handle = handle.clone();
                std::thread::spawn(move || loop {
                    if TERMINATION_REQUESTED.swap(false, Ordering::SeqCst) {
                        shutdown_client(&signal_handle);
                        signal_handle.exit(0);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                });
            }
            let settings = state_settings(&app.state::<AppState>());
            publish(&handle);

            if let Some(window) = app.get_webview_window("main") {
                if std::env::args().any(|arg| arg == "--hidden") {
                    let _ = window.hide();
                }
                dev_server_fallback(&handle, &window);
            }

            if settings.start_client_on_launch {
                let handle = handle.clone();
                let settings = settings.clone();
                tauri::async_runtime::spawn(async move {
                    // 客户端需要 root，这一步会弹一次密码框；失败了不阻断应用启动
                    let started = tauri::async_runtime::spawn_blocking(move || {
                        vpn::start_service(&settings).map(|spawned| {
                            if spawned {
                                vpn::mark_managed();
                            }
                        })
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(softether_tray_lib::i18n::tr(
                            "启动客户端任务失败",
                            "The client start task failed",
                        )
                        .to_string())
                    });
                    if let Err(err) = started {
                        notify(
                            &handle,
                            &Settings::default(),
                            softether_tray_lib::i18n::tr("客户端未启动", "Client did not start"),
                            &err,
                        );
                    }
                    publish(&handle);
                });
            }

            if settings.routes.auto_connect {
                if let Some(name) = settings.active_account.clone() {
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        // 等桌面和网络就绪再连，否则开机瞬间的失败会被当成“服务坏了”。
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        connect_account(&handle, name);
                    });
                }
            }
            Ok(())
        })
        // 托盘常驻：关窗口只隐藏，真正退出只走托盘菜单。
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running SoftEther Tray");
}
