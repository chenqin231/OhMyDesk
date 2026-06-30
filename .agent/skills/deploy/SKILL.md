---
name: deploy
description: Use when deploying OhMyDesk competition/demo edition to a remote server with Docker, especially local image build, docker save/scp upload, SQLite volume persistence, fixed JWT secret, direct 8765 exposure, and smoke verification.
---

# Deploy

## 核心原则

服务器不编译。所有构建在本机完成：`docker build` → `docker save | gzip` → `scp` → 远端 `docker load` → `docker run`。

比赛版不依赖 BT、MySQL、nginx。容器直接开放 `8765`，浏览器访问 `http://<服务器IP>:8765/`，WebSocket 使用 `ws://<服务器IP>:8765/ws`。

默认使用 `--network host`，避免小型比赛服务器上 Docker bridge / DNAT 与云网络策略冲突。确需 bridge 端口映射时可设置 `NETWORK_MODE=bridge`，脚本会改用 `-p 0.0.0.0:8765:8765`。

优先运行脚本：

```bash
.agent/skills/deploy/scripts/deploy-ohmydesk-docker.sh
```

可通过环境变量覆盖默认值：

```bash
SSH_TARGET=chin@rc.guoziweb.com DOMAIN=rc.guoziweb.com IMAGE_TAG=ohmydesk:20260627 .agent/skills/deploy/scripts/deploy-ohmydesk-docker.sh
```

## 前置条件

- 本机可运行 Docker，并能 SSH 到服务器。
- 服务器已安装 Docker。
- 项目根目录存在 `Dockerfile`。
- 生产/演示环境必须有固定 `OHMYDESK_JWT_SECRET`。脚本会优先复用远端 `/opt/ohmydesk/ohmydesk.env` 中已有值；没有则生成一次并写入远端 env 文件。

## 标准流程

1. 检查当前仓库状态，确认没有会被 Docker build 忽略的必要文件。
2. 本地构建镜像：`docker build -t "$IMAGE_TAG" .`。
3. 本地冒烟：启动临时容器，验证 `/`。
4. 导出镜像 tar.gz 到 `/tmp/ohmydesk-deploy/`。
5. 上传到服务器 `/tmp/` 并 `docker load`。
6. 在服务器生成 `/opt/ohmydesk/ohmydesk.env`，权限 `600`。
7. 重建 `ohmydesk` 容器：

```bash
docker rm -f ohmydesk || true
docker run -d --name ohmydesk --restart unless-stopped \
  --network host \
  -v ohmydesk-data:/app/data \
  --env-file /opt/ohmydesk/ohmydesk.env \
  "$IMAGE_TAG"
```

8. 验证：

```bash
curl -fsS http://127.0.0.1:8765/
docker logs --tail=80 ohmydesk
docker inspect ohmydesk --format '{{.State.Health.Status}}'
```

日志期望包含 `SQLite 就绪，审计存储已启用`。若看到 `OHMYDESK_JWT_SECRET 未设置`，说明部署不合格，必须补固定密钥后重启。

## 常见问题

- `/api/endpoints` 返回 401：正常，登录功能启用后 API 需要 Bearer token；健康检查应访问 `/`。
- `permission denied` 读取 env 文件：远端 env 文件必须对运行 `docker run` 的 SSH 用户可读，建议 `chown <ssh_user> && chmod 600`。
- 服务器资源不足：不要在服务器 `docker build`，只 `docker load`。
- 需要 HTTPS/wss：比赛版默认不配；如要域名 TLS，再额外使用 `bt-api-site-ssl` 或 Caddy。

## 修改管理员密码（后台 CLI）

系统设置网页改密入口已下线，改密只能在服务器端用 CLI 子命令（仅持服务器 shell 的管理员可操作）：

Docker 部署（镜像内服务端二进制为 `/app/server`）：

```bash
docker exec ohmydesk /app/server set-password '新密码' [--user 新用户名]
docker restart ohmydesk   # 重启生效
```

裸机部署（二进制名为 `server`，即 `target/release/server`）：

```bash
DATABASE_URL=sqlite:/app/data/ohmydesk.db ./server set-password '新密码'
# 重启 server 进程生效
```

- 不传 `--user` 时仅改密码、保留现有用户名（无持久化则默认 `admin`）。
- 写入 SQLite `settings` 表（bcrypt 哈希）；改密**需重启 server 进程**才会被运行中的实例加载。
- 无可写 DB 时命令报错退出（改密不会静默失败）。
