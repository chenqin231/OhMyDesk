---
name: release
description: Use when releasing OhMyDesk to production (生产发版) — merging to master, pushing/tagging to trigger CI client artifacts, deploying the server via Docker, and publishing the signed auto-update channel + download page. Covers the full server+client production release SOP, edge cases, gotchas, rollback, and explicit STOP-and-ask gates.
---

# Release（生产发版 SOP）

把「合并 master → 推送 → 上 CI → 发布客户端 + 服务器至生产」固化为可复跑流程。**本 SOP 由一次真实 v0.3.0 发版 + 踩坑提炼**（见末尾「实战教训」）。

## 🔴 铁律
- **缺信息就停**：版本号、签名私钥、推送权限、协议兼容性任一不明，**立即停下问人**，绝不猜着发。
- **生产脚本先本地干跑**：任何要 `sudo`/原子切换线上文件的脚本（尤其 `publish-update.sh`），**先在本地把签名/校验链跑通**再上生产。本 SOP 的脚本就是这样捉出 2 个致命 bug 的。
- **download.html 已与仓库分叉**：只用 `sudo sed -i` 改特定行，**绝不**用仓库版本整文件覆盖。
- **不破坏向后兼容**：服务器先发时，新服务端必须接住仍在线的旧客户端。

## 两条独立轨道
| 轨道 | 触发条件 | 机制 | 是否走 CI |
|---|---|---|---|
| **A 服务器** | `src/server/`、`src/admin-web/`、影响服务端的 `protocol`、`Dockerfile` 有改动 | 本机 Docker：build→save→scp→load→run（[[deploy]] skill） | ❌ 本地 |
| **B 客户端** | `src/client/`、影响客户端的 `protocol` 有改动 | 打 tag → `release.yml` 出 6 平台产物 → 自更新通道 + 下载页 | ✅ CI |

**同时含两者：先 A 后 B（先服务器后客户端）**——中转/控制面先就位。

---

## 前置条件（开始前自检）
- 在 `master`、工作区干净（`git status`）。
- 生产签名私钥在 `~/.ohmydesk/update-sec.key`（0600，无口令），公钥 `~/.ohmydesk/update-pub.key`，且**已异地备份**（丢了永远签不了更新）。公钥必须与客户端 `src/client/src/update.rs` 的 `UPDATE_PUBKEY` 一致。
- 免密 sudo SSH 到 `chin@rc.guoziweb.com`；本机 Docker 可用；`gh` 已登录（`gh auth status`）。
- 工具齐：`rsign`（rsign2）、`jq`、`gzip`、`dpkg-deb`、`sha256sum`、`curl`。
- **推送目标 = `origin`**（同时推 `git.guoziweb.com` 自托源 + `github` CI 仓）。CI（`release.yml`）只在 github 触发。

---

## 决策树
```
有服务端改动? ──是──> 跑 Track A（先）
        └─否
有客户端改动? ──是──> 跑 Track B
        └─否──> 无需发版
两者都有 ──> A 完成并验证 → 再 B
纯服务端改动 ──> 只 A，无需 bump 版本 / 打 tag（服务器不走 CI）
```

---

## Track A — 服务器发布（Docker，不走 CI）

1. 确认服务端改动已合入 `master`。
2. 跑部署脚本（细节见 [[deploy]] skill）：
   ```bash
   SSH_TARGET=chin@rc.guoziweb.com DOMAIN=rc.guoziweb.com \
     bash .agent/skills/deploy/scripts/deploy-ohmydesk-docker.sh
   ```
   要点：服务器**不编译**（本机 build→`docker save|gzip`→scp→远端 `docker load`→`docker run`）；复用远端 `/opt/ohmydesk/ohmydesk.env` 的固定 `OHMYDESK_JWT_SECRET` + `ohmydesk-data` 卷；`--network host` 跑 8765，nginx TLS 在前。
3. 验证：
   ```bash
   ssh chin@rc.guoziweb.com 'curl -fsS http://127.0.0.1:8765/ >/dev/null && docker inspect ohmydesk --format "{{.State.Health.Status}}" && docker logs --tail 40 ohmydesk | grep "SQLite 就绪"'
   curl -fsS https://rc.guoziweb.com/ -o /dev/null -w "%{http_code}\n"   # nginx TLS 入口 200
   ```
   日志须含 `SQLite 就绪，审计存储已启用`；若见 `OHMYDESK_JWT_SECRET 未设置` = 不合格，补固定密钥重启。
4. admin-web 改动随镜像发布（`web_dir=/app/web`），无需单独传。
5. 改管理员密码（如需）：`docker exec ohmydesk /app/server set-password '新密码' [--user 名] && docker restart ohmydesk`。

---

## Track B — 客户端发布（CI + 自更新 + 下载页）

### B1 合并与版本
1. 客户端改动 `--no-ff` 合入 `master`。
2. **bump 版本**：改根 `Cargo.toml` 的 `[workspace.package] version`（这是客户端 `agent_version`=`CARGO_PKG_VERSION` 的来源，自更新比对靠它）。
3. `cargo build -p client`（刷新 `Cargo.lock`），`git add Cargo.toml Cargo.lock && git commit`。

### B2 发版前硬门
```bash
cargo test --workspace                                  # 全绿
cargo check -p client --target x86_64-pc-windows-gnu    # Windows 交叉编译门
```
两者必过。`UPDATE_PUBKEY` 误填会让自更新静默 fail-closed → 靠 `内置公钥_可解析` 单测兜底（已在 B2 的 test 里）。

### B3 推送 + 打 tag（触发 CI）
```bash
git fetch origin && [ "$(git rev-list --count master..origin/master)" = 0 ] || { echo "落后，先处理"; exit 1; }  # ff 保护
git push origin master                                  # 推两个镜像
git tag -a v<ver> -m "v<ver> ..."
git push origin v<ver>                                  # 触发 release.yml
```
> 边界：`origin` 推两个镜像。若 `git.guoziweb.com` 推送失败而 github 成功，CI 仍会触发；补推 github 即可（`git push github master v<ver>`）。

### B4 等 CI 并核对
```bash
RUN=$(gh run list -R chenqin231/OhMyDesk --workflow=release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch $RUN -R chenqin231/OhMyDesk --exit-status --interval 30      # 后台运行，约 10–20 分钟
gh run view $RUN -R chenqin231/OhMyDesk --json jobs --jq '.jobs[]|"\(.name): \(.conclusion)"'
gh release view v<ver> -R chenqin231/OhMyDesk --json assets --jq '.assets[].name'
```
龙芯 job `skipped` 属正常（需自托管 runner）。

### B5 下载产物 + 自建 Linux 免安装 tarball
```bash
gh release download v<ver> -R chenqin231/OhMyDesk -D /tmp/<ver>          # 必须带 -R
# CI 不产 Linux 免安装 tarball：从 amd64 deb 取 glibc2.28 二进制（勿用本机 cargo —— glibc 不匹配信创基线）
dpkg-deb -x /tmp/<ver>/ohmydesk-client_<ver>_amd64.deb /tmp/debx
mkdir -p /tmp/stage/ohmydesk-client && cp /tmp/debx/usr/bin/ohmydesk-client /tmp/stage/ohmydesk-client/
cp packaging/download/linux-pkg/* /tmp/stage/ohmydesk-client/
tar -C /tmp/stage -czf /tmp/stage/ohmydesk-client-linux-x86_64.tar.gz ohmydesk-client
```

### B6 发布自更新通道（先本地干跑！）
**先本地验证签名链**（不碰服务器）：用 `rsign sign -W -s <sec> -x <sig> <FILE>` 产 `.minisig`，再 `rsign verify -P "$(tail -1 <pub>)" -x <sig> <FILE>` 必须 `Signature ... verified`。
然后跑发布脚本（**默认先灰度**）：
```bash
OHMYDESK_UPDATE_SECKEY=~/.ohmydesk/update-sec.key \
OHMYDESK_UPDATE_PUBKEY=~/.ohmydesk/update-pub.key \
  bash packaging/download/publish-update.sh <ver> /tmp/<ver>/ohmydesk-client-windows-x86_64.exe --rollout 10
```
脚本：版本化 exe+sha+gzip → 生成全平台 `latest.json` → rsign 签名 → scp 先产物后清单 → **远端验收**（拉远端 exe 落文件比对 sha/size + 验签）→ 原子 `mv latest.json` → 刷 win 稳定别名。
**灰度推进**：观察少量客服机正常后，改 `--rollout 50`→`100` 重跑（重签发布）。`--enabled false` 可秒级全网暂停。

### B7 刷新全平台下载页产物
```bash
# stage 齐：deb×2、自建 linux tarball、macOS tar.gz×2、macOS dmg×2，以及 manifest 用的版本化 macOS 名
cp /tmp/<ver>/ohmydesk-client-macos-arm64.tar.gz /tmp/stage/ohmydesk-client-macos-arm64-<ver>.tar.gz
scp /tmp/stage/* /tmp/<ver>/*.deb /tmp/<ver>/*macos* chin@rc.guoziweb.com:/tmp/up/
ssh chin@rc.guoziweb.com 'D=/www/wwwroot/rc.guoziweb.com/downloads
  sudo cp /tmp/up/* $D/ && sudo chown root:root $D/* && sudo chmod 644 $D/*
  sudo rm -f $D/ohmydesk-client_<旧ver>_amd64.deb $D/ohmydesk-client_<旧ver>_arm64.deb'
```
> **macOS 版本化名必传**：`latest.json` 的 `macos_arm64.url` 指向 `*-<ver>.tar.gz`（CI 产的是不带版本名），不传则该提示链 404。

### B8 改 download.html（分叉文件，只改特定行）
先备份，再 `sudo sed -i` 按锚点改：版本 pill `v<旧>`→`v<新>`、deb URL 版本号、各 size（按 **MiB** 计：`字节/1048576`）。例：
```bash
ssh chin@rc.guoziweb.com 'H=/www/wwwroot/rc.guoziweb.com/download.html; sudo cp $H $H.bak.<ver>.$(date +%s)
  sudo sed -i -e "s|v<旧>|v<新>|" \
    -e "s|ohmydesk-client_<旧>_amd64.deb|ohmydesk-client_<新>_amd64.deb|" \
    -e "s|ohmydesk-client_<旧>_arm64.deb|ohmydesk-client_<新>_arm64.deb|" \
    -e "/windows-x86_64.exe\"/ s|\"<旧size>\"|\"<新size>\"|" \
    -e "/_amd64.deb\"/ s|\"<旧>\"|\"<新>\"|" -e "/_arm64.deb\"/ s|\"<旧>\"|\"<新>\"|" \
    -e "/macos-arm64.tar.gz\"/ s|\"<旧>\"|\"<新>\"|" -e "/macos-x86_64.tar.gz\"/ s|\"<旧>\"|\"<新>\"|" \
    -e "/linux-x86_64.tar.gz\"/ s|\"<旧>\"|\"<新>\"|" $H'
```

### B9 最终验收
```bash
B=https://rc.guoziweb.com/downloads
for u in latest.json latest.json.minisig ohmydesk-client-windows-x86_64-<ver>.exe \
  ohmydesk-client_<ver>_amd64.deb ohmydesk-client-linux-x86_64.tar.gz \
  ohmydesk-client-macos-arm64-<ver>.tar.gz; do
  curl -sk -o /dev/null -w "$u %{http_code}\n" $B/$u; done
curl -sk -o /dev/null -w "旧deb应404: %{http_code}\n" $B/ohmydesk-client_<旧ver>_amd64.deb
# 模拟客户端验签（最关键）
curl -fsS $B/latest.json -o /tmp/L.json && curl -fsS $B/latest.json.minisig -o /tmp/L.sig
rsign verify -P "$(tail -1 ~/.ohmydesk/update-pub.key)" -x /tmp/L.sig /tmp/L.json   # 必须 verified
```
全 200、旧 deb 404、`/download` 页新版本、`latest.json` 验签通过 = 客户端必能验。

### B10 收尾
- 更新 [[release-publish-process]] / [[five-feature-roadmap]] 记忆。
- **Bootstrap 提醒**：带新能力的首版若现网客户端无对应逻辑（如首个自更新版），仍需手动装一次。

---

## ⚠️ 实战教训（必看，都是真踩过的坑）
| 坑 | 后果 | 对策 |
|---|---|---|
| bash 变量装二进制 | `\0` 截断 → sha/size 验收假败 | 远端验收**落临时文件**再 `sha256sum`/`stat` |
| rsign2 CLI 语法 | `-m` 不存在、报 try --help | 文件是**位置参数**：`rsign sign -W -s <sec> -x <sig> <FILE>` / `rsign verify -P <pub串> -x <sig> <FILE>`；无口令密钥签名带 `-W` |
| 直接跑未测脚本上生产 | 中途 sudo 失败/污染线上 | **先本地干跑**签名链；脚本 `set -e` + 远端验收在原子切换前 |
| download.html 整文件覆盖 | 抹掉生产分叉内容 | 只 `sudo sed -i` 改特定行，先备份 |
| 本机 cargo 建 Linux tarball | glibc2.39 不兼容信创 2.28 | 从 amd64 deb `dpkg-deb -x` 取二进制 |
| 漏传 macOS 版本化名 | manifest 提示链 404 | 额外传 `*-macos-arm64-<ver>.tar.gz` |
| UPDATE_PUBKEY 误填 | 自更新静默 fail-closed | `内置公钥_可解析` 单测兜底；发版前确认公钥=私钥配对 |
| 丢生产私钥 | 永远签不了更新（客户端只认这把公钥） | 异地备份；轮换公钥需发一次客户端更新 |
| `/downloads/`、`download.html` root:root | 写入 permission denied | scp 到 `/tmp` 再 `sudo cp/mv` + `chown root:root` |

## 回滚
- **客户端坏包**：① `--enabled false` 重签发布（秒级全网停更）；② 用旧版本号+旧 sha+`--allow-downgrade` 重签（已升机器自动降级，保留上版 exe 在 `/downloads/`）；③ 灰度把坏机面压小。
- **服务器**：远端 `docker images` 找上一镜像 `docker run` 回滚，或从上一 commit 重 build 部署。

## STOP-and-ask（遇到立即停下问人）
- 新版本号该是多少（语义不明）。
- 生产签名私钥缺失/疑似泄露/要不要加口令。
- 服务端+客户端同发但**协议兼容性存疑**。
- CI 任一 job 失败。
- `publish-update.sh` 远端验收/验签失败（**绝不**跳过强发）。
- `git.guoziweb.com` 推送被拒（权限/分叉）。

相关：[[deploy]]（服务器细节）、[[release-publish-process]] 记忆、`packaging/download/publish-update.sh`、`.github/workflows/release.yml`。
