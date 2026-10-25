#!/usr/bin/env bash
# 安装 sudoers 白名单，让 softether-tray 免密启停 SoftEther 客户端。
#
# 只放开 vpnclient start / stop 两个参数组合，不放开任意命令。
# 卸载：sudo bash Tools/install-sudoers.sh --remove
set -euo pipefail

RULE_FILE=/etc/sudoers.d/softether-tray
VPNCLIENT="$(readlink -f /home/cja/Downloads/vpnclient/vpnclient 2>/dev/null || true)"

if [[ "${1:-}" == "--remove" ]]; then
  rm -f "$RULE_FILE"
  echo "已移除 $RULE_FILE"
  exit 0
fi

if [[ -z "$VPNCLIENT" || ! -x "$VPNCLIENT" ]]; then
  echo "找不到 vpnclient 可执行文件，请先在设置里指定客户端目录" >&2
  exit 1
fi

# 用 visudo -c 校验后再落盘，避免写坏 sudoers 把 sudo 锁死
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

cat > "$TMP" <<EOF
# softether-tray: 允许免密启停 SoftEther VPN 客户端（仅这两条命令）
Cmnd_Alias SOFTETHER_TRAY = $VPNCLIENT start, $VPNCLIENT stop
ALL ALL=(root) NOPASSWD: SOFTETHER_TRAY
EOF

if ! visudo -c -f "$TMP" >/dev/null 2>&1; then
  echo "sudoers 语法校验失败，未做任何修改" >&2
  exit 1
fi

install -m 0440 -o root -g root "$TMP" "$RULE_FILE"
echo "已安装 $RULE_FILE"
echo
echo "规则内容："
sed 's/^/  /' "$RULE_FILE"
echo
echo "验证：sudo -n $VPNCLIENT stop 应当不再询问密码（服务未运行时也无妨）"
