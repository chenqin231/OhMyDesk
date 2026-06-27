//! 硬件资产采集：sysinfo → [`EndpointInfo`]。
//!
//! 坑点（见 references/sysinfo-quinn.md）：
//! - CPU usage 本采集不需要（只取型号/核数），故无需刷新 2 次；但仍按规范刷新拿稳定 brand。
//! - MAC/IP 直接走 `Networks`，取首个非 loopback、非全零 MAC 的 IPv4 物理网卡。
//! - GPU 是 unreleased feature（0.39.5 无 `Gpus`），本版统一降级 `None`，由 server/admin 容忍。

use protocol::{CpuArch, CpuInfo, EndpointInfo, OsInfo, OsKind, RamInfo};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

/// 采集本机硬件 → EndpointInfo。`user_name` 为使用人（命令行传入）。
pub fn collect(user_name: &str) -> EndpointInfo {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    // CPU brand 在首刷即可得；usage 本场景不取，省去第二次刷新的 sleep。
    sys.refresh_cpu_all();

    let cpus = sys.cpus();
    let cpu = CpuInfo {
        model: cpus
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default(),
        cores: cpus.len() as u32,
        arch: map_arch(std::env::consts::ARCH),
    };
    let os_name = System::name().unwrap_or_default();
    let os = OsInfo {
        kind: map_os(&os_name),
        name: format!("{} {}", os_name, System::os_version().unwrap_or_default())
            .trim()
            .to_string(),
    };
    let (ip, mac) = primary_nic();
    let id = stable_machine_id(&mac);

    EndpointInfo {
        id,
        name: user_name.to_string(),
        department: None,
        ip,
        mac,
        os,
        cpu,
        ram: RamInfo {
            total: sys.total_memory(),
            used: sys.used_memory(),
        },
        gpu: None, // unreleased feature，统一降级
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// 终端稳定唯一 ID：以**首个物理网卡 MAC**（全球唯一、硬件绑定）确定性映射成 **9 位数字码**。
/// 9 位数字（1_0000_0000~9_9999_9999）便于用户读/记/口头报（模式B 配对用，对标 TeamViewer/RustDesk）。
/// 同一台机器重启/重连恒定不变（修复每次启动在资产列表新增幽灵记录）；MAC 天然区分不同机器。
/// MAC 不可用（占位/空，极少）→ 退回主机名，再退回随机数，始终是 9 位数字。
/// 用 `DefaultHasher::new()`（固定种子 SipHash，跨进程稳定），不可用 `RandomState`（每进程随机种子）。
fn stable_machine_id(mac: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mac = mac.trim().to_lowercase();
    let seed = if mac.is_empty() || mac == "00:00:00:00:00:00" {
        System::host_name()
            .map(|h| h.trim().to_lowercase())
            .filter(|s| !s.is_empty())
    } else {
        Some(mac)
    };
    let raw = match seed {
        Some(s) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut hasher);
            hasher.finish()
        }
        // 无 MAC 也无主机名（极少）：随机 u64，仍映射 9 位数字
        None => uuid::Uuid::new_v4().as_u128() as u64,
    };
    // 映射到固定 9 位（首位非 0）
    format!("{}", 100_000_000 + raw % 900_000_000)
}

/// 当前内存用量（心跳上报）。轻量刷新，不触进程。
pub fn cur_ram() -> RamInfo {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_memory();
    RamInfo {
        total: sys.total_memory(),
        used: sys.used_memory(),
    }
}

/// 一张网卡的判别信息（用于确定性挑主网卡，可单测）。
#[derive(Clone)]
struct Nic {
    name: String,
    mac: String,
    ipv4: Option<String>,
}

/// 确定性挑「首个物理网卡」→ (ipv4, mac)。跨次启动稳定（修复 IP/MAC 乱跳 + ID 漂移的根因）。
fn primary_nic() -> (String, String) {
    let nets = Networks::new_with_refreshed_list();
    let cands: Vec<Nic> = nets
        .iter()
        .map(|(name, d)| Nic {
            name: name.clone(),
            mac: d.mac_address().to_string(),
            ipv4: d
                .ip_networks()
                .iter()
                .find(|n| !n.addr.is_loopback() && n.addr.is_ipv4())
                .map(|n| n.addr.to_string()),
        })
        .collect();
    match pick_primary(cands) {
        Some(n) => (n.ipv4.unwrap_or_else(|| "0.0.0.0".into()), n.mac),
        None => ("0.0.0.0".into(), "00:00:00:00:00:00".into()),
    }
}

/// 纯函数挑主网卡（可单测）：过滤零 MAC，按 非虚拟 → 全局烧录MAC → 有活动IPv4 → 网卡名 排序取首。
/// 全确定，与网卡枚举顺序无关，保证同机每次启动选到同一张卡。
fn pick_primary(mut nics: Vec<Nic>) -> Option<Nic> {
    nics.retain(|n| n.mac.len() >= 2 && n.mac != "00:00:00:00:00:00");
    nics.sort_by(|a, b| {
        is_virtual_nic(&a.name)
            .cmp(&is_virtual_nic(&b.name)) // 非虚拟(false) 在前
            .then(mac_universal(&b.mac).cmp(&mac_universal(&a.mac))) // 全局烧录MAC 在前
            .then(b.ipv4.is_some().cmp(&a.ipv4.is_some())) // 有 IPv4 在前
            .then(a.name.cmp(&b.name)) // 名字升序定序
    });
    nics.into_iter().next()
}

/// MAC 首字节本地管理位(0x02)为 0 → 全局烧录（真实硬件）；为 1 → 本地管理（随机/多数虚拟网卡）。
fn mac_universal(mac: &str) -> bool {
    u8::from_str_radix(mac.get(..2).unwrap_or("00"), 16)
        .map(|b| b & 0x02 == 0)
        .unwrap_or(false)
}

/// 网卡名是否明显属于虚拟适配器（WSL/虚拟机/容器/隧道等）。best-effort 跨平台，仅作排序降权。
fn is_virtual_nic(name: &str) -> bool {
    let n = name.to_lowercase();
    const VIRTUAL: &[&str] = &[
        "loopback",
        "vethernet",
        "veth",
        "wsl",
        "vmware",
        "virtualbox",
        "vbox",
        "hyper-v",
        "hyperv",
        "docker",
        "tailscale",
        "zerotier",
        "tun",
        "tap",
        "vpn",
        "bluetooth",
        "virtual",
        "isatap",
        "teredo",
    ];
    VIRTUAL.iter().any(|p| n.contains(p))
}

/// 运行架构字符串（`std::env::consts::ARCH`）→ 协议 CpuArch。
pub fn map_arch(arch: &str) -> CpuArch {
    match arch {
        "loongarch64" => CpuArch::LoongArch,
        "aarch64" => CpuArch::Aarch64,
        "x86_64" => CpuArch::X86_64,
        _ => CpuArch::Other,
    }
}

/// OS 名称（`System::name()`）→ 协议 OsKind，信创优先识别。
pub fn map_os(name: &str) -> OsKind {
    let n = name.to_lowercase();
    if n.contains("kylin") || name.contains("麒麟") {
        OsKind::Kylin
    } else if n.contains("uos") || n.contains("统信") || n.contains("deepin") {
        OsKind::Uos
    } else if n.contains("windows") {
        OsKind::Windows
    } else if n.contains("linux") {
        OsKind::Linux
    } else {
        OsKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_arch_信创架构() {
        assert_eq!(map_arch("loongarch64"), CpuArch::LoongArch);
        assert_eq!(map_arch("aarch64"), CpuArch::Aarch64);
        assert_eq!(map_arch("x86_64"), CpuArch::X86_64);
        assert_eq!(map_arch("riscv64"), CpuArch::Other);
    }

    #[test]
    fn map_os_信创系统识别() {
        assert_eq!(map_os("Kylin Linux"), OsKind::Kylin);
        assert_eq!(map_os("麒麟操作系统"), OsKind::Kylin);
        assert_eq!(map_os("UOS"), OsKind::Uos);
        assert_eq!(map_os("统信桌面"), OsKind::Uos);
        assert_eq!(map_os("deepin"), OsKind::Uos);
        assert_eq!(map_os("Windows 11"), OsKind::Windows);
        assert_eq!(map_os("Ubuntu Linux"), OsKind::Linux);
        assert_eq!(map_os("FreeBSD"), OsKind::Other);
    }

    fn mk_nic(name: &str, mac: &str, ip: Option<&str>) -> Nic {
        Nic {
            name: name.into(),
            mac: mac.into(),
            ipv4: ip.map(|s| s.into()),
        }
    }

    #[test]
    fn 选主网卡_物理全局mac优先_与枚举顺序无关() {
        let nics = vec![
            mk_nic("vEthernet (WSL)", "00:15:5d:01:02:03", Some("172.27.112.1")), // 虚拟名
            mk_nic("以太网", "a4:bb:6d:11:22:33", Some("192.168.3.10")),          // 物理全局MAC
            mk_nic(
                "VMware Network Adapter VMnet8",
                "00:50:56:c0:00:08",
                Some("192.168.48.1"),
            ), // 虚拟名
        ];
        let want = "a4:bb:6d:11:22:33";
        assert_eq!(
            pick_primary(nics.clone()).unwrap().mac,
            want,
            "应选物理网卡"
        );
        // 打乱枚举顺序仍选同一张 → 确定性（修复 IP/MAC/ID 乱跳）
        let mut shuffled = nics.clone();
        shuffled.rotate_left(2);
        assert_eq!(pick_primary(shuffled).unwrap().mac, want);
        let mut rev = nics;
        rev.reverse();
        assert_eq!(pick_primary(rev).unwrap().mac, want);
        // 全零 MAC 被过滤
        assert!(pick_primary(vec![mk_nic("x", "00:00:00:00:00:00", None)]).is_none());
    }

    #[test]
    fn 稳定id_九位数字_同mac一致_不同mac不同() {
        let a = stable_machine_id("a4:bb:6d:11:22:33");
        // 9 位纯数字，便于用户读/报（模式B 配对）
        assert_eq!(a.len(), 9, "9 位数字码");
        assert!(a.chars().all(|c| c.is_ascii_digit()));
        // 同 MAC 恒定（大小写归一）→ 同机重启不再新增记录
        assert_eq!(a, stable_machine_id("A4:BB:6D:11:22:33"));
        // 不同 MAC → 不同码
        assert_ne!(stable_machine_id("aa:00:00:00:00:01"), a);
        // 占位 MAC → 退回主机名派生，仍是 9 位数字
        let h = stable_machine_id("00:00:00:00:00:00");
        assert_eq!(h.len(), 9);
        assert!(h.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn collect_产出有效结构() {
        // 真实硬件采集：不强校验具体值（依赖运行环境），只验字段不崩、id 为 9 位数字、版本号非空。
        let info = collect("测试机");
        assert_eq!(info.id.len(), 9, "9 位数字 ID");
        assert!(info.id.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(info.name, "测试机");
        assert!(!info.agent_version.is_empty());
        assert!(info.ram.total > 0, "应采到真实总内存");
        assert!(info.gpu.is_none(), "GPU 本版降级 None");
    }
}
