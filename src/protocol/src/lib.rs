//! OhMyDesk 协议契约 —— 三端单一事实源（Rust server/client + ts-rs 导出给 admin-web）。
//!
//! 裁决回流：A-1（department）、C-1（audit type 统一含 input）、W0-1（RegisterAck）、
//! W0-2（audit 枚举定死）、W0-3（`#[serde(tag="type")]` 内部 tag，type 在 payload 内）。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ── 终端实体 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EndpointInfo {
    pub id: String,
    pub name: String,               // 使用人
    pub department: Option<String>, // 部门（裁决 A-1：B 端管理 / 「谁在控财务部电脑」）
    pub ip: String,
    pub mac: String,
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub ram: RamInfo,
    pub gpu: Option<GpuInfo>,
    pub agent_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OsInfo {
    pub name: String,
    pub kind: OsKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum OsKind {
    Kylin,
    Uos,
    Windows,
    Linux,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CpuInfo {
    pub model: String,
    pub cores: u32,
    pub arch: CpuArch,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CpuArch {
    LoongArch,
    Aarch64,
    X86_64,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RamInfo {
    pub total: u64,
    pub used: u64,
} // 字节

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GpuInfo {
    pub model: String,
    pub vram: Option<u64>,
}

/// OS 或 CPU 任一为国产即判定信创
pub fn is_xinchuang(os: &OsInfo, cpu: &CpuInfo) -> bool {
    matches!(os.kind, OsKind::Kylin | OsKind::Uos)
        || matches!(cpu.arch, CpuArch::LoongArch | CpuArch::Aarch64)
}

pub fn xinchuang_label(os: &OsInfo, cpu: &CpuInfo) -> String {
    if !is_xinchuang(os, cpu) {
        return "非信创".into();
    }
    let os_s = match os.kind {
        OsKind::Kylin => "麒麟",
        OsKind::Uos => "统信",
        _ => "其他",
    };
    let cpu_s = match cpu.arch {
        CpuArch::LoongArch => "龙芯",
        CpuArch::Aarch64 => "鲲鹏",
        _ => "其他",
    };
    format!("信创·{os_s}·{cpu_s}")
}

impl EndpointInfo {
    pub fn sample() -> Self {
        EndpointInfo {
            id: "ep-001".into(),
            name: "张伟".into(),
            department: Some("财务部".into()),
            ip: "10.0.0.21".into(),
            mac: "AA:BB:CC:00:00:21".into(),
            os: OsInfo {
                name: "麒麟 V10".into(),
                kind: OsKind::Kylin,
            },
            cpu: CpuInfo {
                model: "Loongson 3A5000".into(),
                cores: 4,
                arch: CpuArch::LoongArch,
            },
            ram: RamInfo {
                total: 16 << 30,
                used: 6 << 30,
            },
            gpu: None,
            agent_version: "0.1.0".into(),
        }
    }
}

// ── 信封 + 消息枚举 ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Envelope {
    pub from: String,
    pub to: Option<String>,
    pub ts: i64,
    pub payload: Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    A,
    B,
}

/// WS 统一消息体；`#[serde(tag="type")]` **内部 tag**——type 在 payload 对象内（非信封顶层），
/// 前端按 `env.payload.type` 判别，Rust 按枚举变体匹配（裁决 W0-3）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Register {
        // Box 化低频的 Register（每连接一次），让高频 Message 变体（Frame/Input）不被 EndpointInfo
        // 撑大 enum；serde/ts-rs 对 Box<T> 透明，JSON 与导出 TS 契约不变（clippy::large_enum_variant）。
        info: Box<EndpointInfo>,
        password: String,
    },
    RegisterAck {
        id: String,
    },
    Heartbeat {
        id: String,
        ram: RamInfo,
    },
    EndpointList {
        endpoints: Vec<EndpointView>,
    },
    ConnectRequest {
        mode: Mode,
        target: String,
        password: Option<String>,
    },
    AuthResult {
        session_id: String,
        ok: bool,
        reason: Option<String>,
    },
    ConnectAck {
        session_id: String,
    },
    Reject {
        session_id: String,
        reason: String,
    },
    Frame {
        session_id: String,
        data: String,
        w: u32,
        h: u32,
        seq: u64,
    },
    Input {
        session_id: String,
        event: InputEvent,
    },
    ScreenshotReq {
        req_id: String,
    },
    ScreenshotResp {
        req_id: String,
        endpoint_id: String,
        data: String,
        w: u32,
        h: u32,
    },
    SessionEnd {
        session_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: u8, down: bool },
    Key { code: String, down: bool },
    Text { text: String },
}

/// 推给管理端的精简视图（含在线态 + 信创标签，不含密码）
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EndpointView {
    pub info: EndpointInfo,
    pub online: bool,
    pub last_seen: i64,
    pub xinchuang: String,
}

// ── 会话与审计实体（ts-rs 导出给前端审计页 + mock；裁决 C-1 audit type 统一）──

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Session {
    pub id: String,
    pub mode: Mode,
    pub from_id: String,
    pub to_id: String,
    pub start_at: i64,
    pub end_at: Option<i64>,
    pub status: SessionStatus,
}

/// 终态语义（Wave 0 钉死）：status 只记会话**最终结果**——拒因细分（密码错 `auth_fail`
/// vs 被控点拒 `reject`）不进 status，查 `AuditLog.kind`；`Active`=进行中，`Ended`=正常结束，
/// `Rejected`=未建立（含两种拒因）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Ended,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuditLog {
    pub id: String,
    pub session_id: String,
    pub ts: i64,
    pub actor_id: String,
    #[serde(rename = "type")]
    pub kind: AuditType, // Rust 关键字 type → 用 kind + serde rename；DB 列名 event_type(B-DB1)
    pub text: String,
}

/// 裁决 C-1：统一为 feature-spec 集合（删 design 的 click、原型的 transfer/error）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AuditType {
    Connect,
    AuthFail,
    Reject,
    Screenshot,
    Input,
    Disconnect,
}

// ── 测试拆分到 src/protocol/src/tests.rs（modularity 规范：测试与实现分离）──
#[cfg(test)]
mod tests;
