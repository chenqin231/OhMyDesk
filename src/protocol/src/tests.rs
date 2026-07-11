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
        actor_user_id: None,
        actor_username: None,
        actor_role: None,
        kind: AuditType::AuthFail,
        text: "密码错误".into(),
    };
    let json = serde_json::to_string(&log).unwrap();
    assert!(json.contains("\"type\":\"auth_fail\""));
}

#[test]
fn session_and_audit_identity_fields_serialize() {
    let session = Session {
        id: "s-1".into(),
        mode: Mode::A,
        from_id: "admin-abc".into(),
        to_id: "ep-1".into(),
        start_at: 100,
        end_at: None,
        status: SessionStatus::Active,
        operator_user_id: Some("u-1".into()),
        operator_username: Some("alice".into()),
        operator_role: Some("operator".into()),
    };
    let json = serde_json::to_value(&session).unwrap();
    assert_eq!(json["operator_user_id"], "u-1");
    assert_eq!(json["operator_username"], "alice");
    assert_eq!(json["operator_role"], "operator");

    let audit = AuditLog {
        id: "a-1".into(),
        session_id: "s-1".into(),
        ts: 101,
        actor_id: "admin-abc".into(),
        actor_user_id: Some("u-1".into()),
        actor_username: Some("alice".into()),
        actor_role: Some("operator".into()),
        kind: AuditType::Connect,
        text: "建立连接".into(),
    };
    let json = serde_json::to_value(&audit).unwrap();
    assert_eq!(json["actor_user_id"], "u-1");
    assert_eq!(json["actor_username"], "alice");
    assert_eq!(json["actor_role"], "operator");
    assert_eq!(json["type"], "connect");
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
            operator_username: Some("caodan".into()),
            mode: Mode::B,
            auto_accept: false,
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"incoming_control\""));
    assert!(json.contains("\"session_id\":\"s-1\""));
    assert!(json.contains("\"operator_username\":\"caodan\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back.payload,
        Message::IncomingControl {
            operator_username: Some(name),
            ..
        } if name == "caodan"
    ));
}

#[test]
fn incoming_control_旧json无_operator_username_兼容() {
    let old_json = r#"{
        "from":"server",
        "to":"ep-2",
        "ts":0,
        "payload":{
            "type":"incoming_control",
            "session_id":"s-1",
            "from":"admin-vazkcy",
            "mode":"b",
            "auto_accept":true
        }
    }"#;

    let back: Envelope = serde_json::from_str(old_json).unwrap();
    assert!(matches!(
        back.payload,
        Message::IncomingControl {
            from,
            operator_username: None,
            ..
        } if from == "admin-vazkcy"
    ));
}

#[test]
fn chat_message_tagged() {
    let env = Envelope {
        from: "ep-1".into(),
        to: None,
        ts: 0,
        payload: Message::ChatMessage {
            session_id: "s-1".into(),
            msg_id: "m-1".into(),
            text: "你好".into(),
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"chat_message\""));
    assert!(json.contains("\"text\":\"你好\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(back.payload, Message::ChatMessage { .. }));
}

#[test]
fn set_capture_tagged() {
    let env = Envelope {
        from: "ep-1".into(),
        to: None,
        ts: 0,
        payload: Message::SetCapture {
            session_id: "s-1".into(),
            active: false,
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"set_capture\""));
    assert!(json.contains("\"active\":false"));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back.payload,
        Message::SetCapture { active: false, .. }
    ));
}

#[test]
fn export_all() {
    let dir = "../admin-web/src/lib/types";
    EndpointInfo::export_all_to(dir).unwrap(); // 带出 OsInfo/CpuInfo/RamInfo/GpuInfo/枚举
    Envelope::export_all_to(dir).unwrap(); // 带出 Message/InputEvent/EndpointView/Mode
    AuditLog::export_all_to(dir).unwrap(); // 审计页/mock 需要（不在 Envelope 链上，须显式）
    Session::export_all_to(dir).unwrap(); // 同上（带出 SessionStatus）
    LoginLogEntry::export_all_to(dir).unwrap(); // 功能②：登录日志类型
}

#[test]
fn set_quality_旧json_三轴字段缺省为none() {
    // 旧主控(≤0.5.0)只发 mode:新被控必须能解析且三轴为 None(回退 mode 旧映射)。
    let json = r#"{"from":"admin-1","to":null,"ts":1719500000,"payload":{"type":"set_quality","session_id":"s-1","mode":"high_quality"}}"#;
    let env: Envelope = serde_json::from_str(json).unwrap();
    match env.payload {
        Message::SetQuality {
            mode,
            resolution,
            clarity,
            fps,
            ..
        } => {
            assert_eq!(mode, QualityMode::HighQuality);
            assert!(resolution.is_none() && clarity.is_none() && fps.is_none());
        }
        _ => panic!("应判别为 SetQuality"),
    }
}

#[test]
fn set_quality_三轴字段_序列化往返() {
    let env = Envelope {
        from: "admin-1".into(),
        to: None,
        ts: 1719500000,
        payload: Message::SetQuality {
            session_id: "s-1".into(),
            mode: QualityMode::Smooth,
            resolution: Some(ResolutionTier::Native),
            clarity: Some(ClarityTier::High),
            fps: Some(FpsTier::Saver),
            adaptive: None,
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(
        json.contains("\"resolution\":\"native\""),
        "snake_case 序列化: {json}"
    );
    assert!(json.contains("\"clarity\":\"high\""));
    assert!(json.contains("\"fps\":\"saver\""));
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back.payload,
        Message::SetQuality {
            resolution: Some(ResolutionTier::Native),
            clarity: Some(ClarityTier::High),
            fps: Some(FpsTier::Saver),
            ..
        }
    ));
}

#[test]
fn set_quality_旧json无adaptive字段_缺省none() {
    // 旧主控不发 adaptive:新被控解析为 None(不改被控当前自适应态,向后兼容)。
    let json = r#"{"from":"a","to":null,"ts":1,"payload":{"type":"set_quality","session_id":"s","mode":"smooth"}}"#;
    let env: Envelope = serde_json::from_str(json).unwrap();
    match env.payload {
        Message::SetQuality { adaptive, .. } => assert!(adaptive.is_none()),
        _ => panic!("应判别为 SetQuality"),
    }
}

#[test]
fn set_quality_adaptive字段_序列化往返() {
    // 主控关自适应:adaptive=Some(false) 往返保真。
    let env = Envelope {
        from: "a".into(),
        to: None,
        ts: 1,
        payload: Message::SetQuality {
            session_id: "s".into(),
            mode: QualityMode::Smooth,
            resolution: None,
            clarity: None,
            fps: None,
            adaptive: Some(false),
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"adaptive\":false"), "序列化含 adaptive: {json}");
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back.payload,
        Message::SetQuality {
            adaptive: Some(false),
            ..
        }
    ));
}

#[test]
fn cursor_update_形状_序列化往返() {
    // 光标同步:被控→主控带形状(RGBA 位图 base64 + 热点 + 尺寸 + id 指纹)。
    let env = Envelope {
        from: "client-1".into(),
        to: None,
        ts: 1719500000,
        payload: Message::CursorUpdate {
            session_id: "s-1".into(),
            x: 100,
            y: 200,
            visible: true,
            shape: Some(CursorShape {
                id: 42,
                hotspot_x: 3,
                hotspot_y: 4,
                w: 32,
                h: 32,
                rgba: "AAAA".into(),
            }),
        },
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(
        json.contains(r#""type":"cursor_update""#),
        "snake_case tag: {json}"
    );
    let back: Envelope = serde_json::from_str(&json).unwrap();
    match back.payload {
        Message::CursorUpdate {
            x,
            y,
            visible,
            shape: Some(s),
            ..
        } => {
            assert_eq!((x, y, visible), (100, 200, true));
            assert_eq!((s.id, s.hotspot_x, s.hotspot_y, s.w, s.h), (42, 3, 4, 32, 32));
            assert_eq!(s.rgba, "AAAA");
        }
        _ => panic!("应判别为 CursorUpdate"),
    }
}

#[test]
fn cursor_update_仅位置_shape缺省none_visible缺省true() {
    // 仅位置更新(形状未变):shape 缺省 None(主控复用缓存)、visible 缺省 true。
    let json = r#"{"from":"c","to":null,"ts":1,"payload":{"type":"cursor_update","session_id":"s","x":5,"y":6}}"#;
    let env: Envelope = serde_json::from_str(json).unwrap();
    match env.payload {
        Message::CursorUpdate {
            x, y, visible, shape, ..
        } => {
            assert_eq!((x, y), (5, 6));
            assert!(shape.is_none());
            assert!(visible, "visible 缺省应为 true");
        }
        _ => panic!("应判别为 CursorUpdate"),
    }
}

/// T026（AC-008-E1）：旧端 EndpointView JSON（无 owner_id 键）反序列化 → owner_id=None，
/// 其余字段完整，不报错不丢消息。owner_id: Option 兜底旧端 serde（历史教训：加字段破坏兼容）。
#[test]
fn endpoint_view_旧json无owner_id_兼容() {
    let info_json = serde_json::to_string(&EndpointInfo::sample()).unwrap();
    // 模拟旧端序列化：不含 owner_id 键。
    let old_json = format!(
        r#"{{"info":{info_json},"online":true,"last_seen":123,"xinchuang":"信创·麒麟·龙芯"}}"#
    );
    let view: EndpointView =
        serde_json::from_str(&old_json).expect("旧 JSON（缺 owner_id）应能反序列化");
    assert_eq!(view.owner_id, None, "缺 owner_id 键 → None");
    assert_eq!(view.info.id, "ep-001", "其余字段完整");
    assert_eq!(view.xinchuang, "信创·麒麟·龙芯");
    assert!(view.online);
    assert_eq!(view.last_seen, 123);
}
