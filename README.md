# SoftEther Tray

一个挂在 Linux 系统托盘的 SoftEther VPN 客户端。官方只提供 Windows 的 Client Manager，Linux 版只有 `vpncmd`，这个应用就是给它补上一层 GUI。

## 能做什么

- 读 SoftEther 客户端里**已有的全部连接配置**，不另存一份影子配置
- 连接 / 断开、新建 / 编辑 / 删除连接配置、创建虚拟网络适配器
- 托盘常驻：显示连接状态，左键打开主窗口，菜单里直接连接或断开任意配置
- 状态面板：隧道 IP、服务器、HUB、MTU、累计流量、上下行速率曲线
- 网络诊断：对任意内网目标做丢包 / 延迟检测
- 连接变化发桌面通知；可选开机自启动与自动连接
- 界面风格切换：Windows 11（Fluent）与 Deepin 25 两套外观，设置页即时预览
- 自定义顶栏（不用系统标题栏）：拖拽移动、双击最大化、边缘拉伸；关闭按钮仍只隐藏到托盘

## 它替你补的两件事

SoftEther 的 Linux 客户端不做这两件事，原 `vpn-tracy` 脚本是用手工路由补的，应用把同样的逻辑做成了可配置的：

1. **网段路由**：连接成功后才下发，让内网网段走隧道
2. **默认路由安全网**：绝不让 VPN 抢走默认路由

另外，断开时会删掉网段路由并清掉网卡地址，避免残留路由把内网流量塞进死隧道。

## UDP 加速

官方 4.44 的 Linux 客户端**没有**关闭 UDP 加速的 `vpncmd` 参数，`AccountDetailSet` 只支持 `/MAXTCP`、`/INTERVAL`、`/TTL`、`/HALF`、`/BRIDGE`、`/MONITOR`、`/NOTRACK`、`/NOQOS`。要关掉只能改客户端配置文件：

```bash
vpn-tracy stop           # 必须先停客户端，否则退出时会写回旧配置
# 把 vpn_client.config 里的 NoUdpAcceleration 改成 true
vpn-tracy
```

这台机器上的 `tet` 配置已经是关闭状态。遇到异常高流量、客户端 CPU 打满、内网丢包时，优先检查这一项。

## 权限

读取状态走当前用户的 `vpncmd`；动路由、启动服务和创建适配器需要 root，统一用 `pkexec` 弹出图形密码框，不会在终端里卡住等 stdin。

## 开发

```bash
npm install
npm run tauri dev
```

构建安装包：

```bash
npm run tauri build
```

产物在 `src-tauri/target/release/bundle/deb/`。

Rust 侧测试：

```bash
cd src-tauri && cargo test --lib
```
