# frame-skip 最小 netem 冒烟门 — 实机验收清单（T16，Go/No-Go）

> **依据**：spec `2026-07-01-dirty-region-frameskip-telemetry-design.md` §5.1。
> **性质**：上线前**人工硬门**。任一项不过 → **No-Go**，先修再发版。
> **填写**：跑完把每格勾选 + 关键日志样本贴到本文件「结论」节，提交。

---

## 0. 准备

**机器**：一台 Linux 物理机/虚机当**被控端**（要有真桌面 X11 会话，**非 Wayland**）；**主控端**用 admin-web（浏览器开远控页）或另一台机器的 Slint 客户端。中转服务端可用生产 `rc.guoziweb.com` 或本地起一个。

**构建被控端**（本分支 `spec-d2-frameskip-and-chat-fix`）：
```bash
cargo build -p client --release   # 产物 target/release/ohmydesk-client（或 client）
```

**找日志/诊断目录**（启动后实际路径，directories crate 决定）：
```bash
find ~/.local/state ~/.local/share -name 'client.log*' 2>/dev/null   # 滚动日志
find ~/.local/state ~/.local/share -path '*OhMyDesk*/diag*' 2>/dev/null  # 诊断包目录
```
被控端启动日志会打印 `渲染模式 mode=... frameskip=... telemetry=...`，确认当前档位。

**被控端跑两种档位**：
- **frameskip ON**（默认）：直接启动，无需环境变量。
- **frameskip OFF**（对照）：`OHMYDESK_FRAMESKIP=0 ./ohmydesk-client`（遥测仍在，sent_Bps 可对比）。
- **legacy 精确旧路径**（验收⑤用）：`./ohmydesk-client --render-mode=legacy-full-frame`，或运行期 UI 热切（Header 的 "S" logo **连点 5 次** → 诊断面板 → 「切回旧画面」）。

---

## 1. 施加网络劣化（在被控端出网网卡 `$IF`，如 `eth0`）

```bash
# 设置：限上行带宽 + 高 RTT（示例 2Mbps + 150ms）
sudo tc qdisc add dev $IF root handle 1: tbf rate 2mbit burst 32kbit latency 400ms
sudo tc qdisc add dev $IF parent 1:1 handle 10: netem delay 150ms

# 查看
tc qdisc show dev $IF

# 清除（每档跑完务必清，再设下一档）
sudo tc qdisc del dev $IF root
```
> 带宽改 `rate 1mbit` / `5mbit` 切换三档。断续重连：测试中途 `sudo tc qdisc del dev $IF root` 拔 5 秒再重设，观察自愈。

---

## 2. 矩阵（每格 frameskip ON / OFF 各一遍）

被控端摆**静态桌面**（不动鼠标）若干秒，再做几次点击/打字，观察画面与日志。

| 上行带宽 | RTT | 抖动/中断 | frameskip ON | frameskip OFF |
|---|---|---|---|---|
| 1 Mbps | +150ms | — | ☐ | ☐ |
| 2 Mbps | +150ms | — | ☐ | ☐ |
| 5 Mbps | +150ms | 断 5s 再恢复 | ☐ | ☐ |

---

## 3. 验收 7 项（任一不过 = No-Go）

- ☐ **① 不崩**：各档画面不黑屏/不卡死；断 5s 重连后 ≤3s（keyframe 周期）自愈。
- ☐ **② 带宽改善可量化**：静态桌面下，frameskip ON 的被控「遥测」日志行 `skip_pct` 高、`sent_Bps` 明显低于 OFF。读法：
  ```bash
  grep '遥测 sid=' <被控滚动日志>   # 看 skip_pct / sent_Bps / effective_fps
  ```
- ☐ **③ 取证字段齐全 + 段定位可用**：劣化下三处日志都有数据，能区分「被控上行饱和」vs「relay→主控」：
  - 被控：`遥测 sid=... stall_p95_ms=... egress_drop=...`
  - 主控（Slint）：`主控遥测 recv_fps=... decode_avg_ms=... drop_stale=... seq_gap=...`
  - server：`grep frame_lane_drop <服务端日志>`（需 server 日志级别含 debug）
- ☐ **④ 异常不漏报**：人为制造卡顿（如降到 1Mbps + 频繁操作）时，被控日志出现 `遥测异常 [...]` 的 WARN，且 diag 目录落 `diag-*.jsonl`。
- ☐ **⑤ legacy 真回退**：切 `--render-mode=legacy-full-frame`，画面与改造前一致，且被控日志**不再出现** `遥测 sid=` 行（证走精确旧路径 `frame_q`，不经新路径/遥测）。
- ☐ **⑥ 运行期热切**：劣化网络下，经 UI 隐藏菜单 frameskip ↔ legacy 热切，**不重启**即生效、画面不中断；日志出现 `UI 热切渲染模式 → ...`。
- ☐ **⑦ 诊断包导出**：
  - 被控：诊断面板「导出诊断包」→ 日志 `手动导出诊断包 <path>`，打开 jsonl 确认含 `stall_ms`/`dirty` 等段字段，**无像素/明文**。
  - admin-web：远控页工具栏「诊断」→ 下载 JSON，确认含 `seq_gap`、**无 `data` 字段**（脱敏）。

---

## 4. 结论（跑完填写）

- **日期 / 跑测人**：
- **矩阵勾选结果**：
- **关键日志样本**（各贴 1-2 行：被控遥测行 / 主控遥测行 / frame_lane_drop / 一次异常 WARN）：
- **7 项验收**：通过 / 不通过（列出不过项）
- **判定**：☐ Go（全部通过，可发版） / ☐ No-Go（待修项：______）

> Go 之后通知我，我走 release 生产发版 SOP（合 master → CI 客户端产物 → Docker 服务端 → 签名自更新 → 下载页）。
