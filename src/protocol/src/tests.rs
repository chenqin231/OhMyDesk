//! protocol 协议契约测试 —— 从 lib.rs 拆出（modularity 规范：测试与实现分离）。
//! 覆盖：信创标识推断、实体/信封/审计序列化往返、payload 内部 tag 判别、ts-rs 导出。

use crate::*;
use ts_rs::TS;

#[test]
fn xinchuang_label_kylin_loongarch() {
    let os = OsInfo {
        name: "麒麟 V10".into(),
        kind: OsKind::Kylin,
    };
    let cpu = CpuInfo {
        model: "Loongson 3A5000".into(),
        cores: 4,
        arch: CpuArch::LoongArch,
    };
    assert!(is_xinchuang(&os, &cpu));
    assert_eq!(xinchuang_label(&os, &cpu), "信创·麒麟·龙芯");
}

#[test]
fn xinchuang_label_windows_x86_is_not() {
    let os = OsInfo {
        name: "Windows 11".into(),
        kind: OsKind::Windows,
    };
    let cpu = CpuInfo {
        model: "Intel i7".into(),
        cores: 8,
        arch: CpuArch::X86_64,
    };
    assert!(!is_xinchuang(&os, &cpu));
}

#[test]
fn endpoint_info_roundtrip() {
    let info = EndpointInfo::sample();
    let json = serde_json::to_string(&info).unwrap();
    let back: EndpointInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(info, back);
}

#[test]
fn envelope_register_roundtrip() {
    let env = Envelope {
        from: "ep-001".into(),
        to: None,
        ts: 1719500000,
        payload: Message::Register {
            info: Box::new(EndpointInfo::sample()),
            password: "123456".into(),
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"register\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(back.payload, Message::Register { .. }));
}

#[test]
fn input_event_tagged() {
    let e = InputEvent::MouseMove { x: 100, y: 200 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"kind\":\"mouse_move\""));
}

#[test]
fn audit_type_field_rename_and_snake() {
    let log = AuditLog {
        id: "a1".into(),
        session_id: "s1".into(),
        ts: 0,
        actor_id: "admin".into(),
        kind: AuditType::AuthFail,
        text: "密码错误".into(),
    };
    let json = serde_json::to_string(&log).unwrap();
    assert!(json.contains("\"type\":\"auth_fail\""));
}

#[test]
fn incoming_control_tagged() {
    let env = Envelope {
        from: "server".into(),
        to: Some("ep-2".into()),
        ts: 0,
        payload: Message::IncomingControl {
            session_id: "s-1".into(),
            from: "ep-1".into(),
            mode: Mode::B,
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"incoming_control\""));
    assert!(json.contains("\"session_id\":\"s-1\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(back.payload, Message::IncomingControl { .. }));
}

#[test]
fn export_all() {
    let dir = "../admin-web/src/lib/types";
    EndpointInfo::export_all_to(dir).unwrap(); // 带出 OsInfo/CpuInfo/RamInfo/GpuInfo/枚举
    Envelope::export_all_to(dir).unwrap(); // 带出 Message/InputEvent/EndpointView/Mode
    AuditLog::export_all_to(dir).unwrap(); // 审计页/mock 需要（不在 Envelope 链上，须显式）
    Session::export_all_to(dir).unwrap(); // 同上（带出 SessionStatus）
}
