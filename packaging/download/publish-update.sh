#!/usr/bin/env bash
# OhMyDesk 在线更新发布：版本化产物 → 离线签名 → 上传 → 远端验收 → 原子切换 latest.json。
# 私钥在本机（rsign2/minisign），绝不进 CI。用法：
#   publish-update.sh <版本号> <win-exe路径> [--rollout 10] [--min-version 0.3.0] [--enabled false] [--allow-downgrade]
# 依赖：rsign(or minisign)、sha256sum、gzip、jq、curl、ssh/scp。需远端 chin@rc.guoziweb.com 免密 sudo。
set -euo pipefail

VER="${1:?需版本号}"; WIN_EXE="${2:?需 Windows exe 路径}"; shift 2
ROLLOUT=100; MINV="null"; ENABLED=true; DOWNGRADE=false
while [ $# -gt 0 ]; do case "$1" in
  --rollout) ROLLOUT="$2"; shift 2;;
  --min-version) MINV="\"$2\""; shift 2;;
  --enabled) ENABLED="$2"; shift 2;;
  --allow-downgrade) DOWNGRADE=true; shift;;
  *) echo "未知参数 $1" >&2; exit 1;;
esac; done

HOST="chin@rc.guoziweb.com"
DLDIR="/www/wwwroot/rc.guoziweb.com/downloads"
SECKEY="${OHMYDESK_UPDATE_SECKEY:?设 OHMYDESK_UPDATE_SECKEY=离线私钥路径}"
PUBKEY="${OHMYDESK_UPDATE_PUBKEY:?设 OHMYDESK_UPDATE_PUBKEY=公钥路径}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

WIN_NAME="ohmydesk-client-windows-x86_64-${VER}.exe"
echo "==> 1/8 版本化 + sha256 + gzip"
cp "$WIN_EXE" "$WORK/$WIN_NAME"
SHA="$(sha256sum "$WORK/$WIN_NAME" | cut -d' ' -f1)"
SIZE="$(stat -c%s "$WORK/$WIN_NAME")"
gzip -9 -kf "$WORK/$WIN_NAME"   # → $WIN_NAME.gz

echo "==> 2/8 生成 latest.json（必须填全平台 asset）"
# Linux/macOS 仅提示：填 url(版本化) + auto:false；如无对应产物则该平台客户端不提示。
jq -n --arg ver "$VER" --arg wurl "https://rc.guoziweb.com/downloads/$WIN_NAME" \
   --arg sha "$SHA" --argjson size "$SIZE" --argjson rollout "$ROLLOUT" \
   --argjson minv "$MINV" --argjson enabled "$ENABLED" --argjson dg "$DOWNGRADE" '
{
  version: $ver,
  assets: {
    windows_x86_64: { url: $wurl, sha256: $sha, size: $size, auto: true },
    linux_x86_64_deb: { url: ("https://rc.guoziweb.com/downloads/ohmydesk-client_" + $ver + "_amd64.deb"), auto: false },
    linux_arm64_deb:  { url: ("https://rc.guoziweb.com/downloads/ohmydesk-client_" + $ver + "_arm64.deb"),  auto: false },
    macos_arm64:      { url: ("https://rc.guoziweb.com/downloads/ohmydesk-client-macos-arm64-" + $ver + ".tar.gz"), auto: false }
  },
  enabled: $enabled, rollout_percent: $rollout, min_version: $minv, allow_downgrade: $dg,
  notes: ("版本 " + $ver)
}' > "$WORK/latest.json"

echo "==> 3/8 离线签名"
rsign sign -s "$SECKEY" -m "$WORK/latest.json" -x "$WORK/latest.json.minisig"

echo "==> 4/8 上传版本化产物(先产物后清单)"
scp "$WORK/$WIN_NAME" "$WORK/$WIN_NAME.gz" "$HOST:/tmp/"
ssh "$HOST" "sudo mv /tmp/$WIN_NAME /tmp/$WIN_NAME.gz $DLDIR/ && sudo chown root:root $DLDIR/$WIN_NAME $DLDIR/$WIN_NAME.gz && sudo chmod 644 $DLDIR/$WIN_NAME $DLDIR/$WIN_NAME.gz"

echo "==> 5/8 远端验收(切清单前)：拉远端 exe(gzip)解压比对 sha/size + 验签"
RMT_SHA="$(curl -fsSk -H 'Accept-Encoding: gzip' --compressed "https://rc.guoziweb.com/downloads/$WIN_NAME" | sha256sum | cut -d' ' -f1)"
[ "$RMT_SHA" = "$SHA" ] || { echo "远端 exe sha256 不符，中止" >&2; exit 1; }
rsign verify -P "$(tail -1 "$PUBKEY")" -m "$WORK/latest.json" -x "$WORK/latest.json.minisig" \
  || minisign -Vm "$WORK/latest.json" -x "$WORK/latest.json.minisig" -p "$PUBKEY"

echo "==> 6/8 上传清单 + 原子切换"
scp "$WORK/latest.json" "$HOST:/tmp/latest.json.tmp"
scp "$WORK/latest.json.minisig" "$HOST:/tmp/latest.json.minisig"
ssh "$HOST" "sudo mv /tmp/latest.json.minisig $DLDIR/latest.json.minisig && sudo mv /tmp/latest.json.tmp $DLDIR/latest.json && sudo chown root:root $DLDIR/latest.json $DLDIR/latest.json.minisig && sudo chmod 644 $DLDIR/latest.json $DLDIR/latest.json.minisig"

echo "==> 7/8 更新下载页稳定别名(供人工下载页) + download.html 特定行"
ssh "$HOST" "sudo cp $DLDIR/$WIN_NAME $DLDIR/ohmydesk-client-windows-x86_64.exe && sudo cp $DLDIR/$WIN_NAME.gz $DLDIR/ohmydesk-client-windows-x86_64.exe.gz"
# download.html 版本/size 用 sudo sed -i 改特定行(禁整文件覆盖，详见 release-publish-process 记忆)

echo "==> 8/8 校验"
curl -fsSk "https://rc.guoziweb.com/downloads/latest.json" | jq -r .version
echo "发布完成：$VER（保留上一版 exe 以备回退）"
