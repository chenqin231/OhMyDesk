---
name: bt-api-site-ssl
description: Use when configuring BT Panel / 宝塔 Linux 面板 from SSH with the BT API, especially creating domains, nginx reverse proxy sites, WebSocket forwarding, Let's Encrypt SSL certificates, and HTTPS verification.
---

# BT API Site SSL

## 核心原则

先 SSH 到服务器本机，再从服务器请求宝塔 API。这样 API 请求来源是 `127.0.0.1`，只需确认宝塔 API 白名单包含 `127.0.0.1`。

宝塔 API 签名固定为：

```python
request_time = str(int(time.time()))
request_token = md5(request_time + md5(api_key))
```

所有请求使用 `POST`，面板地址优先探测：

```text
https://127.0.0.1:<panel_port>
https://127.0.0.1:<panel_port>/<security_path>
```

## 工作流

1. 读取本地 `.env` 的 `BT`，不要打印密钥。
2. SSH 登录服务器，确认 `bt default` 输出的面板端口和安全入口。
3. 用 `/system?action=GetSystemTotal` 探测 API 基础 URL；若返回 `IP校验失败`，先让用户或你用 sudo 修改 `/www/server/panel/config/api.json` 的 `limit_addr`，加入 `127.0.0.1`。
4. 创建站点：

```text
POST /site?action=AddSite
webname={"domain":"rc.guoziweb.com","domainlist":[],"count":0}
path=/www/wwwroot/rc.guoziweb.com
type=PHP
version=00
port=80
ftp=false
sql=false
```

若站点已存在，不重复创建；读取现有 vhost 并备份。

5. 配置 nginx 反代到本机应用端口，必须保留 WebSocket 头：

```nginx
proxy_pass http://127.0.0.1:8765;
proxy_http_version 1.1;
proxy_set_header Host $host;
proxy_set_header X-Real-IP $remote_addr;
proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
proxy_set_header X-Forwarded-Proto $scheme;
proxy_set_header Upgrade $http_upgrade;
proxy_set_header Connection "upgrade";
proxy_read_timeout 3600s;
proxy_send_timeout 3600s;
```

6. 申请 Let's Encrypt 证书：

```text
POST /ssl/cert/apply_for_cert
domains=["rc.guoziweb.com"]
auth_type=http
auth_to=["rc.guoziweb.com"]
auto_wildcard=0
```

成功返回 `cert`、`root`、`private_key` 后部署到站点：

```text
POST /ssl/cert/SetBatchCertToSite
BatchInfo=[{"siteName":"rc.guoziweb.com"}]
privkey=<private_key>
fullchain=<cert + root>
```

7. 若宝塔没有写入 443 vhost，手动补 `server { listen 443 ssl; http2 on; ... }`，证书路径使用：

```text
/www/server/panel/vhost/cert/<domain>/fullchain.pem
/www/server/panel/vhost/cert/<domain>/privkey.pem
```

8. 验证：

```bash
nginx -t
curl -fsS https://<domain>/api/endpoints
curl -i -N --http1.1 \
  -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://<domain>/ws --max-time 5
echo | openssl s_client -servername <domain> -connect <domain>:443 2>/dev/null | openssl x509 -noout -subject -issuer -dates
```

WebSocket 验证期望看到 `101 Switching Protocols`。

## 注意事项

- 不要在输出中打印 `BT`、数据库密码、证书私钥。
- 修改 vhost 前先备份：`cp site.conf site.conf.bak.$(date +%Y%m%d%H%M%S)`。
- 若 HTTPS 返回旧页面，通常是 443 server 块未命中新反代配置。
- 若 HTTP 正常、HTTPS 404，检查 nginx `-T` 中该域名是否有 `listen 443`。
