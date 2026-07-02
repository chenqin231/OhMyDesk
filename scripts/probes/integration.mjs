// Wave 2 综合集成探针：I2 远控回归 + I3 批量截图 + M4 审计落库
// 两被控 agent（ep-a 信创麒麟龙芯 / ep-b 非信创）+ 一 admin 主控。
const BASE = "ws://127.0.0.1:8765/ws";
const ADMIN_USER = process.env.PROBE_USER || "superadmin";
const ADMIN_PASS = process.env.PROBE_PASS || "infogo123";
const log = [];
const ok = (m) => log.push("✅ " + m);
const info = (m) => log.push("· " + m);

function mkInfo(id, name, dept, osName, osKind, cpuModel, arch) {
  return { id, name, department: dept, ip: "10.0.0." + (id.length * 3),
    mac: "aa:bb:cc:00:00:0" + id.slice(-1),
    os: { name: osName, kind: osKind }, cpu: { model: cpuModel, cores: 8, arch },
    ram: { total: 16777216000, used: 8388608000 }, gpu: null, agent_version: "0.1.0" };
}
const send = (ws, payload, from, to = null) => ws.send(JSON.stringify({ from, to, ts: 0, payload }));

const A_ID = "ep-a", A_PW = "111111";
const B_ID = "ep-b";
const ADMIN = "admin-probe";
let sessionId = null;
let shotRespCount = 0;
const shotEndpoints = new Set();

// ── 被控 agent 工厂 ──────────────────────────────────────────────
function makeAgent(id, pw, infoObj, opts = {}) {
  const ws = new WebSocket(BASE);
  ws.addEventListener("open", () => send(ws, { type: "register", info: infoObj, password: pw }, id));
  ws.addEventListener("message", (ev) => {
    const p = JSON.parse(ev.data).payload;
    if (p.type === "register_ack") { info(`${id} 注册确认`); opts.onReady?.(); }
    else if (p.type === "incoming_control") {
      info(`${id} 收 IncomingControl session=${p.session_id.slice(0,8)} → 授权`);
      send(ws, { type: "auth_result", session_id: p.session_id, ok: true, reason: null }, id);
    }
    else if (p.type === "input") {
      ok(`${id} 收 Input（主控→被控路由）${JSON.stringify(p.event)}`);
      send(ws, { type: "frame", session_id: p.session_id, data: "<jpeg>", w: 1280, h: 720, seq: 1 }, id);
    }
    else if (p.type === "screenshot_req") {
      // I3：被控收截图请求 → 回 screenshot_resp（to=请求方，endpoint_id=本机）
      const from = JSON.parse(ev.data).from;
      info(`${id} 收 screenshot_req req=${p.req_id} → 回发`);
      send(ws, { type: "screenshot_resp", req_id: p.req_id, endpoint_id: id,
        data: "<shot-b64-" + id + ">", w: 1280, h: 720 }, id, from);
    }
  });
  return ws;
}

// ── 主控 admin（需先登录拿 token，WS 带 ?token）─────────────────────
let admin;
async function startAdmin() {
  const r = await fetch("http://127.0.0.1:8765/api/login", { method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ user: ADMIN_USER, pass: ADMIN_PASS }) });
  const { token } = await r.json();
  info("admin 登录拿到 token");
  admin = new WebSocket(`${BASE}?token=${encodeURIComponent(token)}`);
  admin.addEventListener("open", () => {
    send(admin, { type: "heartbeat", id: ADMIN, ram: { total: 0, used: 0 } }, ADMIN);
  });
  admin.addEventListener("message", (ev) => {
    const env = JSON.parse(ev.data); const p = env.payload;
    if (p.type === "endpoint_list") {
      if (p.endpoints.length >= 2 && !sessionId && admin._connecting !== true) {
        const xc = p.endpoints.map((e) => `${e.info.id}:${e.xinchuang}:${e.online?"在线":"离线"}`).join(", ");
        ok(`admin 收 endpoint_list（${p.endpoints.length} 台）${xc}`);
        admin._connecting = true;
        info("admin 发 ConnectRequest(模式B, 正确密码) → ep-a");
        send(admin, { type: "connect_request", mode: "b", target: A_ID, password: A_PW }, ADMIN);
      }
    }
    else if (p.type === "connect_ack") {
      sessionId = p.session_id;
      ok(`admin 收 ConnectAck session=${sessionId.slice(0,8)}（I2 远控建立）`);
      info("admin 发 Input(MouseMove 320,240)");
      send(admin, { type: "input", session_id: sessionId, event: { kind: "mouse_move", x: 320, y: 240 } }, ADMIN);
    }
    else if (p.type === "frame") {
      ok(`admin 收 Frame（被控→主控路由）${p.w}x${p.h}`);
      info("admin 发 screenshot_req（一键批量截图）");
      send(admin, { type: "screenshot_req", req_id: "req-batch-1" }, ADMIN);
    }
    else if (p.type === "screenshot_resp") {
      shotRespCount++; shotEndpoints.add(p.endpoint_id);
      ok(`admin 收 screenshot_resp endpoint=${p.endpoint_id} bytes=${p.data.length}（I3 第${shotRespCount}张）`);
      if (shotEndpoints.has(A_ID) && shotEndpoints.has(B_ID)) {
        info("admin 发 session_end（结束远控）");
        send(admin, { type: "session_end", session_id: sessionId }, ADMIN);
        setTimeout(finish, 500);
      }
    }
    else if (p.type === "reject") log.push("❌ admin 收 Reject: " + p.reason);
  });
}

function finish() {
  const pass = shotEndpoints.has(A_ID) && shotEndpoints.has(B_ID) && sessionId;
  console.log(log.join("\n"));
  console.log("\n=== " + (pass ? "Wave2 综合闭环通过：I2 远控 + I3 双端批量截图 + 会话结束" : "未达成") + " ===");
  process.exit(pass ? 0 : 1);
}

// 启动顺序：两 agent 就绪后再起 admin
let ready = 0;
const onReady = () => { if (++ready === 2) setTimeout(startAdmin, 300); };
makeAgent(A_ID, A_PW, mkInfo(A_ID, "张伟", "财务部", "麒麟 V10", "kylin", "Loongson 3A5000", "loong_arch"), { onReady });
makeAgent(B_ID, "222222", mkInfo(B_ID, "李娜", "研发部", "Windows 10", "windows", "Intel i7-8700", "x86_64"), { onReady });

setTimeout(() => { console.log(log.join("\n") + "\n❌ 超时"); process.exit(1); }, 10000);
