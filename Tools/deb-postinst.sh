#!/bin/sh
# softether-tray 安装后配置。
#
# deb 的 dpkg 只能把文件放到固定位置，做不了这些事，所以放在这里：
#   1. 给内置客户端脚本可执行权限
#   2. 装 polkit 授权（让连接/断开不再弹密码框）
#
# 刻意**不自动启动** vpnclient：装了不等于要连，启动由用户决定。
set -e

CLIENT_DIR=/usr/lib/softether-tray/client
HELPER=/usr/libexec/softether-tray/softether-tray-net

# 1. 内置客户端：dpkg 会保留包里的权限，但保险起见再设一次。
#    这些文件必须是 0755，否则普通用户连 --help 都跑不了。
if [ -d "$CLIENT_DIR" ]; then
    chmod 0755 "$CLIENT_DIR" 2>/dev/null || true
    for f in vpnclient vpncmd; do
        [ -f "$CLIENT_DIR/$f" ] && chmod 0755 "$CLIENT_DIR/$f" 2>/dev/null || true
    done
    for f in hamcore.se2 lang.config; do
        [ -f "$CLIENT_DIR/$f" ] && chmod 0644 "$CLIENT_DIR/$f" 2>/dev/null || true
    done
fi

# 2. polkit：policy 已由包安装到 /usr/share/polkit-1/actions/。
#    免密规则单独控制 —— 默认装上（体验优先），但用户随时可以删掉这一行文件。
if [ -f /usr/share/polkit-1/actions/cn.softether.tray.net.policy ]; then
    if command -v chmod >/dev/null 2>&1; then
        chmod 0644 /usr/share/polkit-1/actions/cn.softether.tray.net.policy 2>/dev/null || true
    fi
fi
if [ -f "$HELPER" ]; then
    chmod 0755 "$HELPER" 2>/dev/null || true
fi

# 3. 让 polkit 重新加载（有些发行版要踢一下才认新 policy）
if command -v systemctl >/dev/null 2>&1; then
    systemctl try-restart polkit >/dev/null 2>&1 || true
fi

echo "softether-tray: 内置 SoftEther 客户端已就位（$CLIENT_DIR）"
exit 0
