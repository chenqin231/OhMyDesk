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

/// 画质档位：高清优先（分辨率/质量高、帧率低）/ 流畅优先（分辨率/质量低、帧率高）。
/// 具体的分辨率/质量/帧率参数由被控端 capture 模块按档位决定，协议只传枚举。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum QualityMode {
    HighQuality,
    Smooth,
}

/// 分辨率档位:采集缩放上限(fit-within 等比,绝不放大)。Native=不缩放,按被控真实屏发送。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionTier {
    R720p,
    R900p,
    R1080p,
    Native,
}

/// 清晰度档位:JPEG 编码质量。Standard=q80,High=q88(q≥90 切 4:4:4 体积翻倍,真机已否决)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ClarityTier {
    Standard,
    High,
}

/// 帧率档位:推帧间隔。Smooth=40ms(~25fps),Standard=66ms(~15fps),Saver=125ms(~8fps)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FpsTier {
    Smooth,
    Standard,
    Saver,
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
        /// WEB 强制远程：免被控端同意直连（仅 admin- 发起方有效，server 端硬校验）。
        #[serde(default)]
        force: bool,
    },
    /// 主控 → server：取消尚未建立的远控申请（此时主控无 session_id）。
    /// server 据 (from, target) 定位挂起会话，向被控端转发 SessionEnd 撤销授权弹窗并结束会话。
    /// 解「主控取消申请后被控端弹窗仍倒计时」。
    CancelRequest {
        target: String,
    },
    /// server → 被控端：有主控发起控制，携带 server 生成的 session_id；
    /// 被控端授权后回 AuthResult 带此 session_id（解 task#8 时序缺口，统一会话 id 来源）。
    IncomingControl {
        session_id: String,
        from: String,
        #[serde(default)]
        operator_username: Option<String>,
        mode: Mode,
        /// true=免同意直连（密码正确/强制），被控端跳过弹框直接进被控态；false=弹框等用户同意。
        #[serde(default)]
        auto_accept: bool,
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
    /// 主控→被控:设置显示参数。三轴新字段(v0.5.x 起)独立控制分辨率/清晰度/帧率;
    /// mode 旧字段保留兼容 ≤0.5.0 被控端(新主控按清晰度映射填写:High→HighQuality)。
    /// 旧主控不发三轴(None),新被控按 mode 兜底展开。按 session 对端路由(同 Input)。
    SetQuality {
        session_id: String,
        mode: QualityMode,
        #[serde(default)]
        resolution: Option<ResolutionTier>,
        #[serde(default)]
        clarity: Option<ClarityTier>,
        #[serde(default)]
        fps: Option<FpsTier>,
    },
    /// 主控→被控:会话内帧推流开关(懒推流——主控仅在「远程桌面」标签需要帧)。
    /// active=false 暂停采集推帧, true 恢复。按 session 对端路由(同 SetQuality);不审计(纯传输优化)。
    SetCapture {
        session_id: String,
        active: bool,
    },
    /// 被控→主控：会话内提示（如 Wayland 无法截屏）。主控端在等待画面处展示，
    /// 把「无限等待第一帧」变成可操作的明确提示。按 session 对端路由（同 Frame）。
    RemoteNotice {
        session_id: String,
        text: String,
    },
    /// 会话内双向纯文本剪贴板同步(主控↔被控)。按 session 对端路由(同 RemoteNotice)。
    ClipboardSync {
        session_id: String,
        text: String,
    },
    /// 被控→主控:光标同步。让主控「看到被控端真实鼠标形状」(箭头/文本 I 型/手型/调整大小…)。
    /// 主控优先在**本地指针位置**渲染此形状并盖住系统光标(零往返延迟);x/y(帧坐标系,同 Frame
    /// 缩放后 w/h)供被控端自身移动光标时兜底定位。形状按 id(RGBA 指纹)缓存:仅在形状变化时带
    /// shape,其余更新 shape=None 复用主控缓存(省带宽,光标≤32×32 仅数 KB)。visible=false 表示
    /// 被控端光标此刻隐藏(如全屏视频/游戏捕获)。按 session 对端路由(同 Frame);旧端不认识 →
    /// Unknown 兜底,不破坏(协议演进不破坏旧端)。
    CursorUpdate {
        session_id: String,
        x: i32,
        y: i32,
        #[serde(default = "default_true")]
        visible: bool,
        #[serde(default)]
        shape: Option<CursorShape>,
    },
    /// 会话内即时消息(双向,主控↔被控)。按 session 对端路由(同 ClipboardSync);
    /// server 转发同时落 AuditType::Chat 审计(全文)。
    ChatMessage {
        session_id: String,
        msg_id: String,
        text: String,
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

    // ── 远程命令执行（一次性；控制方→被控方→回执，按 session 路由）────────────
    /// 控制方下发一次性命令；被控方用系统 shell 执行（Win `cmd /C` / Linux `sh -c`）。
    ExecRequest {
        session_id: String,
        exec_id: String,
        command: String,
        timeout_ms: u32,
    },
    /// 被控方回传执行结果（stdout/stderr 各截断 64KB，truncated 标记是否被截）。
    ExecResult {
        session_id: String,
        exec_id: String,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        truncated: bool,
        duration_ms: u32,
    },

    // ── 文件传输（分块流；push=控制方→被控方，pull=被控方→控制方）──────────────
    /// 发起一次传输：dir=push 时控制方紧接着发 FileChunk；dir=pull 时为被控方对
    /// FilePullRequest 的回流首包。
    FileOpen {
        session_id: String,
        transfer_id: String,
        name: String,
        size: u64,
        dir: FileDir,
        /// push 下发的目标目录（控制方在远端文件浏览器里选定的当前目录）；
        /// None 或非法时被控端回退到固定接收目录 recv_dir。pull 回流时恒为 None。
        #[serde(default)]
        dest: Option<String>,
    },
    /// 数据块；data 为该块 base64，last=true 表示末块。
    FileChunk {
        session_id: String,
        transfer_id: String,
        seq: u64,
        data: String,
        last: bool,
    },
    /// 控制方请求取回被控方某路径文件 → 被控方以 FileOpen{dir:pull}+FileChunk 回流。
    FilePullRequest {
        session_id: String,
        transfer_id: String,
        path: String,
    },
    /// 传输失败/被拒（超限、路径不可读、目录穿越等）。
    FileError {
        session_id: String,
        transfer_id: String,
        reason: String,
    },
    /// 被控方收齐 push 文件并落盘后回执，path 为被控端最终绝对路径（解「下发不知去向」）。
    FileDone {
        session_id: String,
        transfer_id: String,
        path: String,
    },

    // ── 远端目录浏览（控制方→被控方列目录→回条目；按 session 路由）──────────────
    /// 控制方请求列出被控方某目录；path 为空时被控方返回默认目录（home）。
    FileListRequest {
        session_id: String,
        transfer_id: String,
        path: String,
    },
    /// 被控方回传目录条目；path 为实际列出的绝对目录（供前端面包屑/上级导航）。
    /// error 非空表示列目录失败（无权限/不存在），entries 为空。
    FileListResp {
        session_id: String,
        transfer_id: String,
        path: String,
        entries: Vec<FileEntry>,
        error: Option<String>,
    },
    /// 未知/未来变体兜底：旧端遇不认识的 `type` 落到此，不再整条 Envelope 反序列化失败。
    /// server 靠原始 text 转发保内容（route_to_peer），新端仍能还原（协议演进不破坏旧端）。
    /// `skip_serializing`：Unknown 只应被反序列化+原样透传，绝不重序列化（否则会丢原字段
    /// 变成 `{"type":"unknown"}`）；标记后误序列化即报错暴露，而非静默损坏。
    #[serde(other, skip_serializing)]
    #[ts(skip)]
    Unknown,
}

/// serde 默认值 helper:缺省字段回退 true(CursorUpdate.visible 无键时视为可见)。
fn default_true() -> bool {
    true
}

/// 光标形状位图(裸 RGBA,base64)。见 Message::CursorUpdate。
/// id = RGBA 像素的 twox-hash 指纹,主控据此缓存去重(同一形状只传一次)。
/// rgba = w*h*4 字节的 base64(未压缩;光标极小且仅形状变化时发,不值得再上 PNG 依赖)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CursorShape {
    pub id: u64,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub w: u32,
    pub h: u32,
    pub rgba: String,
}

/// 远端目录中的一个条目（文件或子目录）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: u8,
        down: bool,
    },
    Key {
        code: String,
        down: bool,
    },
    Text {
        text: String,
    },
    /// 鼠标滚轮。dx/dy 单位为滚轮"格"(notches,非像素);dy>0 向上、dx>0 向右。
    Scroll {
        dx: i32,
        dy: i32,
    },
    /// 未知/未来变体兜底：旧端遇不认识的 `kind` 落到此，不再整条失败(见 Message::Unknown)。
    /// `skip_serializing` 理由同 Message::Unknown：只反序列化+原样透传，误序列化即报错暴露。
    #[serde(other, skip_serializing)]
    #[ts(skip)]
    Unknown,
}

/// 文件传输方向：push=控制方推给被控方，pull=被控方回流给控制方。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FileDir {
    Push,
    Pull,
}

/// 推给管理端的精简视图（含在线态 + 信创标签，不含密码）
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EndpointView {
    pub info: EndpointInfo,
    pub online: bool,
    pub last_seen: i64,
    pub xinchuang: String,
    /// 归属账号 user_id（服务端从连接 JWT 派生，非客户端自报）。
    /// `Option` 兜底旧端 serde：旧 JSON 缺此键反序列化为 None，不破坏兼容。
    pub owner_id: Option<String>,
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
    pub operator_user_id: Option<String>,
    pub operator_username: Option<String>,
    pub operator_role: Option<String>,
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
    pub actor_user_id: Option<String>,
    pub actor_username: Option<String>,
    pub actor_role: Option<String>,
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
    Command,      // 远程命令执行
    FileTransfer, // 文件传输（下发/取回）
    Chat,         // 会话内即时消息
}

/// 管理员登录日志条目（功能②；server → admin-web，ts-rs 导出）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoginLogEntry {
    pub id: i64,
    pub ts: i64,
    pub username: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub success: bool,
    pub reason: Option<String>,
}

// ── 测试拆分到 src/protocol/src/tests.rs（modularity 规范：测试与实现分离）──
#[cfg(test)]
mod tests;
