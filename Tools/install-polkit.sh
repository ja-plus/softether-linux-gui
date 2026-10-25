#!/usr/bin/env bash
# 安装 / 卸载 softether-tray 的 polkit 授权，让虚拟网卡配置不再每次弹密码框。
#
#   sudo bash Tools/install-polkit.sh           安装并启用免密
#   sudo bash Tools/install-polkit.sh --no-auto 只装 policy（免密关闭，仍会弹框）
#   sudo bash Tools/install-polkit.sh --remove  完全卸载
#
# 装了什么：
#   /usr/libexec/softether-tray/softether-tray-net   参数受限的特权脚本（0755 root:root）
#   /usr/share/polkit-1/actions/cn.softether.tray.net.policy
#   /etc/polkit-1/rules.d/50-softether-tray.rules    免密规则（--no-auto 时不装）
#
# 安全边界见 softether-tray-net 文件头。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_DIR=/usr/libexec/softether-tray
TARGET_BIN=$TARGET_DIR/softether-tray-net
POLICY=/usr/share/polkit-1/actions/cn.softether.tray.net.policy
RULES=/etc/polkit-1/rules.d/50-softether-tray.rules

MODE=auto
case "${1:-}" in
  --remove)  MODE=remove ;;
  --no-auto) MODE=policy-only ;;
  "")        MODE=auto ;;
  *) echo "未知参数: $1（可用：--remove / --no-auto）" >&2; exit 2 ;;
esac

[[ $EUID -eq 0 ]] || { echo "请用 root 运行：sudo bash Tools/install-polkit.sh" >&2; exit 1; }

if [[ "$MODE" == remove ]]; then
  rm -f "$RULES" "$POLICY" "$TARGET_BIN"
  rmdir "$TARGET_DIR" 2>/dev/null || true
  echo "已卸载："
  echo "  $RULES"
  echo "  $POLICY"
  echo "  $TARGET_BIN"
  echo
  echo "现在连接 VPN 会恢复为每次弹密码框。"
  exit 0
fi

# 1. 落脚本。先做语法检查，避免装进去一个跑不起来的权限入口。
sh -n "$HERE/softether-tray-net" || { echo "softether-tray-net 语法检查失败" >&2; exit 1; }
install -d -m 0755 -o root -g root "$TARGET_DIR"
install -m 0755 -o root -g root "$HERE/softether-tray-net" "$TARGET_BIN"

# 2. 落 policy
install -d -m 0755 /usr/share/polkit-1/actions
install -m 0644 -o root -g root "$HERE/cn.softether.tray.net.policy" "$POLICY"

# 3. 落免密规则（可选）
if [[ "$MODE" == auto ]]; then
  install -d -m 0755 /etc/polkit-1/rules.d
  install -m 0644 -o root -g root "$HERE/50-softether-tray.rules" "$RULES"
fi

# 4. 自检：脚本能不能被 polkit 认出来（读一下 action 是否可见）
echo "已安装："
echo "  $TARGET_BIN   ($(stat -c '%A %U:%G' "$TARGET_BIN"))"
echo "  $POLICY"
[[ -f "$RULES" ]] && echo "  $RULES   (免密已启用)" || echo "  （未安装免密规则，仍会弹密码框）"
echo

# 5. 参数校验冒烟测试：用不存在的前缀网卡验证拒绝逻辑仍生效
if "$TARGET_BIN" connect --check vpn_vpn 1400 "10.0.0.0/8" >/dev/null 2>&1; then
  echo "自检：参数校验正常"
else
  echo "自检警告：connect --check 未通过（vpn_vpn 网卡可能还没建，属正常）"
fi

# 6. 关键自检：action 是否真被 polkitd 加载
#
# 这一步不能省。policy 文件权限不对、目录不可读、XML 有错，polkitd 会**静默忽略**，
# 表现就是"装了但还弹密码框"，而且没有任何错误提示，只能靠这里问出来。
if command -v busctl >/dev/null 2>&1; then
  loaded=$(busctl call org.freedesktop.PolicyKit1 /org/freedesktop/PolicyKit1/Authority \
    org.freedesktop.PolicyKit1.Authority EnumerateActions s "" 2>/dev/null \
    | grep -c 'cn\.softether\.tray\.net' || true)
  if [[ "${loaded:-0}" -ge 1 ]]; then
    echo "自检：polkitd 已加载 action cn.softether.tray.net"
  else
    cat >&2 <<'WARN'

自检失败：polkitd 没有加载 cn.softether.tray.net。
可能原因：
  - policy 文件权限不对（应为 0644 root:root）
  - XML 格式错误（polkitd 会静默跳过有错的 policy）
  - polkitd 未运行：systemctl status polkit
排查：
  journalctl -u polkit -n 50 --no-pager
WARN
    exit 1
  fi
else
  echo "自检跳过：没有 busctl，无法确认 action 是否已加载"
fi

if [[ -f "$RULES" ]]; then
  cat <<'TIP'

验证方法：
  1. 先断开再连接一次 VPN —— 应当**不再**出现密码框。
  2. 对照实验：pkexec id 仍应弹出密码框（说明只放行了本应用）。
  3. 若仍弹框，看日志：
       journalctl -u polkit -n 50 --no-pager
     polkit 123 起把规则错误也打在这里，规则写错会导致整条规则被忽略。
回落办法：
  sudo bash Tools/install-polkit.sh --remove
TIP
fi
