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
        name: format!(
            "{} {}",
            os_name,
            System::os_version().unwrap_or_default()
        )
        .trim()
        .to_string(),
    };
    let (ip, mac) = first_nic();

    EndpointInfo {
        id: format!("ep-{}", &uuid::Uuid::new_v4().to_string()[..8]),
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

/// 取首个有效物理网卡的 (ipv4, mac)；找不到回退占位值。
fn first_nic() -> (String, String) {
    let nets = Networks::new_with_refreshed_list();
    for (_, d) in &nets {
        let mac = d.mac_address().to_string();
        if mac == "00:00:00:00:00:00" {
            continue;
        }
        if let Some(ipn) = d
            .ip_networks()
            .iter()
            .find(|n| !n.addr.is_loopback() && n.addr.is_ipv4())
        {
            return (ipn.addr.to_string(), mac);
        }
    }
    ("0.0.0.0".into(), "00:00:00:00:00:00".into())
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

    #[test]
    fn collect_产出有效结构() {
        // 真实硬件采集：不强校验具体值（依赖运行环境），只验字段不崩、id 前缀、版本号非空。
        let info = collect("测试机");
        assert!(info.id.starts_with("ep-"));
        assert_eq!(info.name, "测试机");
        assert!(!info.agent_version.is_empty());
        assert!(info.ram.total > 0, "应采到真实总内存");
        assert!(info.gpu.is_none(), "GPU 本版降级 None");
    }
}
