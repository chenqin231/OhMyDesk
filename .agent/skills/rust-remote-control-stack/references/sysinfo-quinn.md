# 硬件采集 + QUIC 网络速查（sysinfo + quinn）

> 结论先行：sysinfo 资产采集走稳定 API 即可；**GPU 是 2026-06 新增、尚未进 0.39.5 release 的 unreleased 能力**，需用 git 依赖 + `gpu` feature。MAC/IP 在新版 sysinfo 已能直接拿，无需额外 crate。quinn 0.11 的 server/client 骨架以 `Endpoint::server/client` + rustls 为准。

---

## 一、sysinfo 速查

### 版本与依赖
- **最新 release**：`0.39.5`（MIT，MSRV `rustc 1.95`）。
- **默认模块化**：`component`/`disk`/`network`/`system`/`user` 都是独立 feature，默认全开。只采集部分信息时按需裁剪可减小体积。

```toml
[dependencies]
sysinfo = "0.39"
```

### System 刷新模式（核心坑都在这）
- `System::new()` 空壳，`System::new_all()` 一次性拉全量。
- **CPU 使用率必须刷新 ≥2 次**，且两次之间要等待 `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`，否则读数为 0 或不准（usage 是差值计算）。
- **避免 `refresh_all()` 滥用**：它会刷新进程等重负载数据。资产上报场景用 `refresh_specifics(RefreshKind)` 精确刷新需要的部分，降开销。
- `Disks`/`Networks`/`Components`/`Gpus` 是**独立对象**，不挂在 `System` 上，各自 `new_with_refreshed_list()`。

```rust
use sysinfo::{System, RefreshKind, CpuRefreshKind, MemoryRefreshKind};

let mut sys = System::new_with_specifics(
    RefreshKind::nothing()
        .with_cpu(CpuRefreshKind::everything())   // 型号/频率/usage
        .with_memory(MemoryRefreshKind::everything()),
);
sys.refresh_cpu_all();
std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
sys.refresh_cpu_all();                             // 第二次才有准的 usage
```

### CPU（型号/核数/频率）
```rust
let cpus = sys.cpus();
let model   = cpus[0].brand();          // &str  CPU 型号
let vendor  = cpus[0].vendor_id();      // &str  厂商
let freq    = cpus[0].frequency();      // u64   MHz
let usage   = cpus[0].cpu_usage();      // f32   百分比
let logical = cpus.len();               // 逻辑核数
let physical = System::physical_core_count(&sys); // Option<usize> 物理核数
```

### 内存（总量/用量，单位字节）
```rust
sys.total_memory();   sys.used_memory();   sys.available_memory();
sys.total_swap();     sys.used_swap();
```

### 磁盘
```rust
use sysinfo::Disks;
let disks = Disks::new_with_refreshed_list();
for d in &disks {
    // d.name() / d.kind()(DiskKind: SSD/HDD) / d.total_space() / d.available_space() / d.file_system()
}
```

### 网络（含 IP/MAC，新版已内置）
```rust
use sysinfo::Networks;
let nets = Networks::new_with_refreshed_list();
for (iface, data) in &nets {
    let mac  = data.mac_address();      // MacAddr      ← MAC 直接拿，无需额外 crate
    let ips  = data.ip_networks();      // &[IpNetwork] ← IP/掩码列表
    let down = data.total_received();   // u64
    let up   = data.total_transmitted();
}
```

### GPU 信息（2026-06 新增，重点 + 注意事项）
**新颖度提醒**：GPU API 在 2026-06-20 官方博客公布，代码在 `main` 分支，**截至 0.39.5 release 尚未发布**（docs.rs 0.39.5 无 `Gpu*` 类型）。要用必须走 git 依赖并启用 `gpu` feature：

```toml
[dependencies]
sysinfo = { git = "https://github.com/GuillaumeGomez/sysinfo", features = ["gpu"] }
# 或等正式发布后：sysinfo = { version = "0.40", features = ["gpu"] }  # 版本号待发布确认
```

API（来自 `src/common/gpu.rs`）：
```rust
use sysinfo::Gpus;
let gpus = Gpus::new_with_refreshed_list()?;   // 返回 Result
for gpu in gpus.list() {                        // &[Gpu]
    gpu.vendor();        // Option<&str>  厂商
    gpu.model();         // Option<&str>  型号/名称
    gpu.total_memory();  // Option<u64>   总显存(字节)
    gpu.used_memory();   // Option<u64>   已用显存
    gpu.usage();         // Option<f32>   利用率(%)
    gpu.pci();           // &PCI          domain/bus/device/function 唯一标识
}
// 增量：gpus.refresh(remove_not_listed_gpus: bool)
```

**平台/国产 GPU 注意**：
- 实现路径：Linux 读 `/sys/class/drm/`，Windows 用 DXGI，macOS 用 IOAccelerator。
- 厂商支持：AMD（直接读 `gpu_busy_percent` 等 sysfs）、NVIDIA（动态加载 NVML 库，需驱动装了 NVML）、**其他厂商走 Vulkan 兜底**。
- **关键限制 / 国产 GPU 影响**：Vulkan 兜底路径**拿不到 GPU 利用率**（`usage()` 会是 `None`），只能拿型号/显存。国产显卡（摩尔线程/景嘉微/海光等）、Intel 核显等非 AMD/NVIDIA 设备多落到 Vulkan 路径，要做好 `usage()` 为 `None` 的降级。
- **无驱动版本字段**：当前公开 API 没有 `driver()`，需要驱动版本要另想办法（读 sysfs / 调厂商工具）。

出处：https://docs.rs/sysinfo/latest/sysinfo/ ｜ GPU 博客 https://blog.guillaume-gomez.fr/articles/2026-06-20+sysinfo:+Getting+GPUs ｜ 源码 https://github.com/GuillaumeGomez/sysinfo/blob/main/src/common/gpu.rs

---

## 二、获取 IP / MAC 的补充结论

- **首选 sysinfo 自带**：`NetworkData::mac_address() -> MacAddr` 和 `ip_networks() -> &[IpNetwork]` 已覆盖 MAC + 每网卡 IP/掩码，本项目无需再引第三方 crate。
- 仅当需要"**本机出口/默认 IP**"（而非遍历所有网卡）时，再考虑：
  - `local-ip-address`：拿默认出站 IP，API 简单（`local_ip()`）。
  - `mac_address`：仅当目标平台 sysinfo 的 MAC 读取有缺失时作兜底（`mac_address::get_mac_address()`）。
- 资产上报建议：遍历 `Networks`，过滤掉 loopback（`127.0.0.1`/`::1`）与全 0 MAC，取第一个 up 且有有效 MAC 的物理网卡作为终端标识。

---

## 三、quinn 速查

### 版本与依赖
- **最新**：`0.11.11`，基于 `rustls 0.23+`。

```toml
[dependencies]
quinn   = "0.11"
rustls  = "0.23"
tokio   = { version = "1", features = ["full"] }
rcgen   = "0.13"   # 自签证书(测试用)
```

### 与 rustls 的关系（必懂）
- quinn 的 TLS 层就是 rustls，**不是可选项**：`ServerConfig`/`ClientConfig` 内部包的是 rustls 配置。
- 便捷构造：`ServerConfig::with_single_cert(certs, key)`（自动包好 rustls）；client 可用 `ClientConfig::try_with_platform_verifier()`（信任系统 CA）。

### Server 最小骨架
```rust
use quinn::{Endpoint, ServerConfig};

async fn run_server(addr: std::net::SocketAddr) -> anyhow::Result<()> {
    // certs: Vec<CertificateDer>, key: PrivateKeyDer —— 生产用正式证书，测试用 rcgen 自签
    let server_config = ServerConfig::with_single_cert(certs, key)?;
    let endpoint = Endpoint::server(server_config, addr)?;

    while let Some(incoming) = endpoint.accept().await {
        let conn = incoming.await?;                 // quinn::Connection
        tokio::spawn(async move {
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                let req = recv.read_to_end(64 * 1024).await?;
                send.write_all(&req).await?;        // echo
                send.finish()?;
            }
            anyhow::Ok(())
        });
    }
    Ok(())
}
```

### Client 最小骨架
```rust
use quinn::{Endpoint, ClientConfig};
use std::net::SocketAddr;

async fn run_client(server: SocketAddr) -> anyhow::Result<()> {
    let mut endpoint = Endpoint::client("[::]:0".parse()?)?;   // 任意本地端口
    endpoint.set_default_client_config(
        ClientConfig::try_with_platform_verifier()?,
    );

    let conn = endpoint.connect(server, "your-server-name")?.await?;  // SNI 名要匹配证书
    let (mut send, mut recv) = conn.open_bi().await?;          // 双向流
    send.write_all(b"ping").await?;
    send.finish()?;
    let resp = recv.read_to_end(64 * 1024).await?;
    Ok(())
}
```

关键 API：`Endpoint::server` / `Endpoint::client` / `endpoint.accept()` / `endpoint.connect(addr, server_name)` / `Connection::open_bi()`（主动开流）/ `Connection::accept_bi()`（被动收流）/ `SendStream::finish()`。

### 本项目"未来传输加速"如何切入
- **定位**：QUIC 跑在 UDP 上，自带 0-RTT/1-RTT、多路复用（无队头阻塞）、连接迁移（IP 切换不断连），适合弱网/移动终端的资产上报与画面分发提速。
- **渐进切入**：
  1. 先保留现有 WS 上报通道，新增一条 quinn 链路做灰度。
  2. **每类数据一条双向流**（`open_bi`）而非每次新建连接，复用单连接多路复用。
  3. 大画面/批量资产用流 + 背压；心跳/小指令用短双向流。
  4. 配 `TransportConfig`（keep-alive、最大并发流、流量窗口）再上生产，避免默认值长连接掉线。

出处：https://docs.rs/quinn/latest/quinn/ ｜ 证书 https://quinn-rs.github.io/quinn/quinn/certificate.html ｜ 示例 https://github.com/quinn-rs/quinn/tree/main/quinn/examples
