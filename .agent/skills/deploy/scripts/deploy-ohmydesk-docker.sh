#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$ROOT_DIR"

SSH_TARGET="${SSH_TARGET:-chin@rc.guoziweb.com}"
DOMAIN="${DOMAIN:-rc.guoziweb.com}"
IMAGE_TAG="${IMAGE_TAG:-ohmydesk:$(date +%Y%m%d%H%M%S)}"
CONTAINER_NAME="${CONTAINER_NAME:-ohmydesk}"
APP_PORT="${APP_PORT:-8765}"
NETWORK_MODE="${NETWORK_MODE:-host}"
PUBLISH_HOST="${PUBLISH_HOST:-0.0.0.0}"
DATA_VOLUME="${DATA_VOLUME:-ohmydesk-data}"
REMOTE_ENV_PATH="${REMOTE_ENV_PATH:-/opt/ohmydesk/ohmydesk.env}"
ARTIFACT_DIR="${ARTIFACT_DIR:-/tmp/ohmydesk-deploy}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "缺少命令: $1" >&2
    exit 1
  }
}

env_value() {
  local key="$1"
  [[ -f .env ]] || return 0
  awk -F= -v k="$key" '$1 == k {print substr($0, index($0, "=") + 1)}' .env
}

generate_secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
    return
  fi
  od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
  printf '\n'
}

need docker
need ssh
need scp
need gzip
need curl

if [[ ! -f Dockerfile ]]; then
  echo "未找到 Dockerfile，请在项目根目录运行。" >&2
  exit 1
fi

mkdir -p "$ARTIFACT_DIR"
SAFE_TAG="${IMAGE_TAG//[:\\/]/-}"
IMAGE_TAR="$ARTIFACT_DIR/${SAFE_TAG}.tar.gz"
LOCAL_ENV_FILE="$(mktemp "${ARTIFACT_DIR}/${SAFE_TAG}.env.XXXXXX")"
REMOTE_TMP_ENV="/tmp/${SAFE_TAG}.env"
cleanup() {
  rm -f "$LOCAL_ENV_FILE"
  docker rm -f "${CONTAINER_NAME}-smoke" >/dev/null 2>&1 || true
}
trap cleanup EXIT
chmod 600 "$LOCAL_ENV_FILE"

REMOTE_JWT_SECRET="$(
  ssh "$SSH_TARGET" "test -f '$REMOTE_ENV_PATH' && sed -n 's/^OHMYDESK_JWT_SECRET=//p' '$REMOTE_ENV_PATH' | head -n 1 || true" 2>/dev/null || true
)"
JWT_SECRET="${OHMYDESK_JWT_SECRET:-${REMOTE_JWT_SECRET:-$(env_value OHMYDESK_JWT_SECRET)}}"
if [[ -z "$JWT_SECRET" ]]; then
  JWT_SECRET="$(generate_secret)"
  echo "==> 未发现既有 OHMYDESK_JWT_SECRET，已生成新固定密钥并写入远端 env"
else
  echo "==> 使用已有 OHMYDESK_JWT_SECRET（不输出密钥）"
fi

{
  printf 'DATABASE_URL=sqlite:/app/data/ohmydesk.db\n'
  printf 'OHMYDESK_JWT_SECRET=%s\n' "$JWT_SECRET"
  # 客户端安装包目录放数据卷（持久化，重建镜像不丢；CI 产物 scp 到此处供 /downloads 提供）。
  printf 'OHMYDESK_DOWNLOAD_DIR=/app/data/downloads\n'
} > "$LOCAL_ENV_FILE"

echo "==> 本地构建镜像: $IMAGE_TAG"
docker build -t "$IMAGE_TAG" .

echo "==> 本地冒烟验证"
docker rm -f "${CONTAINER_NAME}-smoke" >/dev/null 2>&1 || true
docker run -d \
  --name "${CONTAINER_NAME}-smoke" \
  -p "127.0.0.1:18765:8765" \
  -e DATABASE_URL=sqlite:/tmp/ohmydesk-smoke.db \
  -e OHMYDESK_JWT_SECRET=smoke-secret \
  "$IMAGE_TAG" >/dev/null
sleep 2
curl -fsS "http://127.0.0.1:18765/" >/dev/null
docker rm -f "${CONTAINER_NAME}-smoke" >/dev/null 2>&1 || true

echo "==> 导出镜像: $IMAGE_TAR"
docker save "$IMAGE_TAG" | gzip -1 > "$IMAGE_TAR"
ls -lh "$IMAGE_TAR"

echo "==> 上传镜像到服务器: $SSH_TARGET"
scp "$IMAGE_TAR" "$SSH_TARGET:/tmp/${SAFE_TAG}.tar.gz"
scp "$LOCAL_ENV_FILE" "$SSH_TARGET:$REMOTE_TMP_ENV"

echo "==> 远端加载并运行容器"
ssh "$SSH_TARGET" \
  "IMAGE_TAG='$IMAGE_TAG' CONTAINER_NAME='$CONTAINER_NAME' REMOTE_ENV_PATH='$REMOTE_ENV_PATH' REMOTE_TMP_ENV='$REMOTE_TMP_ENV' SAFE_TAG='$SAFE_TAG' APP_PORT='$APP_PORT' NETWORK_MODE='$NETWORK_MODE' PUBLISH_HOST='$PUBLISH_HOST' DATA_VOLUME='$DATA_VOLUME' bash -s" <<'REMOTE'
set -Eeuo pipefail

docker load -i "/tmp/${SAFE_TAG}.tar.gz"

sudo -n mkdir -p "$(dirname "$REMOTE_ENV_PATH")"
sudo -n install -m 600 -o "$(id -un)" -g "$(id -gn)" "$REMOTE_TMP_ENV" "$REMOTE_ENV_PATH"
rm -f "$REMOTE_TMP_ENV"
chmod 600 "$REMOTE_ENV_PATH"

docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
if [[ "$NETWORK_MODE" == "host" ]]; then
  docker run -d \
    --name "$CONTAINER_NAME" \
    --restart unless-stopped \
    --network host \
    -v "${DATA_VOLUME}:/app/data" \
    --env-file "$REMOTE_ENV_PATH" \
    "$IMAGE_TAG" >/dev/null
else
  docker run -d \
    --name "$CONTAINER_NAME" \
    --restart unless-stopped \
    -p "${PUBLISH_HOST}:${APP_PORT}:8765" \
    -v "${DATA_VOLUME}:/app/data" \
    --env-file "$REMOTE_ENV_PATH" \
    "$IMAGE_TAG" >/dev/null
fi

sleep 3
curl -fsS "http://127.0.0.1:${APP_PORT}/" >/dev/null
docker ps --filter "name=${CONTAINER_NAME}" --format '{{.Names}} {{.Image}} {{.Status}}'
docker logs --tail=80 "$CONTAINER_NAME" | sed -E 's#OHMYDESK_JWT_SECRET=[^ ]+#OHMYDESK_JWT_SECRET=<masked>#g'
REMOTE

cleanup
trap - EXIT
echo "==> 部署完成: http://${DOMAIN}:${APP_PORT}/ 或 http://<服务器IP>:${APP_PORT}/"
