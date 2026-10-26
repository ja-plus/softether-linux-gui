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
            softether_tray_lib::i18n::tr("SoftEther VPN · 未连接", "SoftEther VPN · Disconnected")
                .to_string()
        });

    // 菜单项先算成 (id, label) 列表：签名要拿它们比对，没变就整轮跳过重建
    let mut account_items: Vec<(String, String)> = Vec::new();
    if overview.accounts.is_empty() {
        account_items.push((
            "no-account".into(),
            softether_tray_lib::i18n::tr("（还没有连接配置）", "(No connection profiles)").into(),
        ));
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
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
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
    let already = settings::vpncmd(&settings, settings::VpncmdAuth::Plain, &["AccountStatusGet", name])
        .map(|o| softether_tray_lib::softether::status_is_connected(&o.stdout))
        .unwrap_or(false);

    // 并发点击和自动连接可能同时进来；真实连接已经存在时不再发第二条 AccountConnect。
    if !already {
        let output = settings::vpncmd(&settings, settings::VpncmdAuth::Plain, &["AccountConnect", name])?;
        if !softether_tray_lib::softether::text_says_success(&output.stdout) {
            return Err(vpn::last_error(&output.stdout));
        }
    }
    // 会话建立是异步的：状态真变已连接之后才动路由，避免往死网卡塞路由。
    let mut connected = already;
    for _ in 0..16 {
        if connected || vpn::account_status(&settings, name).is_some() {
            connected = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if !connected {
        return Err(softether_tray_lib::i18n::tr(
            "等待隧道建立超时，请查看客户端日志或网络状态",
            "Timed out waiting for the tunnel; check the client log or network status.",
        )
        .into());
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
                decorate_dhcp_error(e, &iface, need_lease)
            })?
        } else {
            let script = settings::connect_routes_script(&iface, &settings.routes, need_lease)?;
            settings::run_privileged(&script).map_err(|e| {
                if need_lease && !settings::has_dhcp_client() {
                    if softether_tray_lib::i18n::is_en() {
                        format!(
                            "{e}\nNo dhclient/dhcpcd is available, so {iface} cannot get an IP. \
                             Install a DHCP client (sudo apt install isc-dhcp-client) or assign a static address to {iface}."
                        )
                    } else {
                        format!(
                            "{e}\n系统没有 dhclient/dhcpcd，{iface} 拿不到 IP。\
                             请安装 DHCP 客户端（sudo apt install isc-dhcp-client），或手动为 {iface} 配静态地址"
                        )
                    }
                } else {
                    e
                }
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
    let result = do_connect(&app, &name);
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
async fn refresh(app: AppHandle) -> Result<Overview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let overview = current_overview(&app);
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
    store_settings(&app.state::<AppState>(), settings.clone())?;
    softether_tray_lib::i18n::set_locale(&settings.locale);
    publish(&app);
    Ok(settings)
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
        // 输出格式必须与 helper 的 probe 完全一致，否则解析层要写两套。
        let script = format!(
            "iface={iface}\nnets={nets_arg}\n\
             gw=$(ip -4 route show dev $iface | awk '/proto kernel/ && / src / {{print $1; exit}}' | cut -d/ -f1 | awk -F. 'BEGIN{{OFS=\".\"}} NF==4{{$4=1; print}}')\n\
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
            run_diagnostics,
            diag_default_target,
            verify_routes,
            set_autostart,
            autostart_enabled,
            env_info,
            config_dir_path,
            open_config_dir,
            list_client_candidates,
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
