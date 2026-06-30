// OhMyDesk 协议级端到端测试:真实 server 二进制 + 真实 WS + 真实 SQLite 审计。
// 覆盖:即时消息(双向)、远程命令、远程文件浏览、懒推流(SetCapture)的会话路由 + 审计落库。
// 不覆盖:Slint GUI 渲染/键鼠注入(需 X11,手动验收)。
// 用法:见同目录 run.sh(负责起停 server)。默认连 127.0.0.1:8765,可用 E2E_HOST 覆盖。
const HOST = process.env.E2E_HOST || "127.0.0.1:8765";
const BASE = `http://${HOST}`;
const WS = `ws://${HOST}/ws`;
const ADMIN_USER = process.env.E2E_USER || "admin";
const ADMIN_PASS = process.env.E2E_PASS || "OhMyDesk@2026";

let passed = 0;
function assert(cond, msg) {
  if (!cond) { console.error("✗ " + msg); throw new Error("断言失败: " + msg); }
  passed++; console.log("✓ " + msg);
}
const epInfo = (id, name) => ({
  id, name, department: null, ip: "10.0.0.9", mac: "AA:BB:CC:00:00:09",
  os: { name: "Linux", kind: "linux" },
  cpu: { model: "x86", cores: 4, arch: "x86_64" },
  ram: { total: 1000, used: 100 }, gpu: null, agent_version: "0.0.0-e2e",
});

function connect(id) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS);
    const pending = []; const backlog = [];
    ws.addEventListener("message", (ev) => {
      const env = JSON.parse(ev.data);
      const i = pending.findIndex((p) => p.pred(env));
      if (i >= 0) { const p = pending.splice(i, 1)[0]; clearTimeout(p.timer); p.resolve(env); }
      else backlog.push(env);
    });
    ws.addEventListener("error", () => reject(new Error("WS 错误: " + id)));
    ws.addEventListener("open", () => resolve({
      send: (payload, to = null) => ws.send(JSON.stringify({ from: id, to, ts: Date.now(), payload })),
      waitFor: (pred, ms = 5000) => new Promise((res, rej) => {
        const i = backlog.findIndex(pred);
        if (i >= 0) { res(backlog.splice(i, 1)[0]); return; }
        const timer = setTimeout(() => rej(new Error(id + " 等待消息超时")), ms);
        pending.push({ pred, resolve: res, timer });
      }),
      close: () => ws.close(),
    }));
  });
}
const isType = (t) => (e) => e.payload.type === t;

async function main() {
  // 1) 两终端注册
  const A = await connect("ep-a");
  A.send({ type: "register", info: epInfo("ep-a", "主控A"), password: "pwA" });
  await A.waitFor(isType("register_ack"));
  const B = await connect("ep-b");
  B.send({ type: "register", info: epInfo("ep-b", "被控B"), password: "pwB" });
  await B.waitFor(isType("register_ack"));
  assert(true, "ep-a / ep-b 注册成功(register_ack)");

  // 2) 模式 B 带密码连接 → 免同意建会话
  A.send({ type: "connect_request", mode: "b", target: "ep-b", password: "pwB", force: false }, "ep-b");
  const ack = await A.waitFor(isType("connect_ack"));
  const sid = ack.payload.session_id;
  await B.waitFor(isType("incoming_control"));
  assert(typeof sid === "string" && sid.length > 0, "会话建立:A 收 connect_ack、B 收 incoming_control");

  // 3) 即时消息——A→B
  const chatText = "你好E2E-" + Date.now();
  A.send({ type: "chat_message", session_id: sid, msg_id: "m1", text: chatText });
  const cm = await B.waitFor(isType("chat_message"));
  assert(cm.payload.text === chatText, "即时消息 A→B 路由到对端(全文一致)");

  // 4) 即时消息——B→A(双向)
  const reply = "收到E2E-" + Date.now();
  B.send({ type: "chat_message", session_id: sid, msg_id: "m2", text: reply });
  const rm = await A.waitFor(isType("chat_message"));
  assert(rm.payload.text === reply, "即时消息 B→A 路由(双向闭环)");

  // 5) 远程命令——A→B
  A.send({ type: "exec_request", session_id: sid, exec_id: "e1", command: "echo e2e", timeout_ms: 5000 });
  const er = await B.waitFor(isType("exec_request"));
  assert(er.payload.command === "echo e2e", "远程命令 ExecRequest 路由到被控端");

  // 6) 远程文件——目录浏览 A→B
  A.send({ type: "file_list_request", session_id: sid, transfer_id: "t1", path: "/tmp" });
  const fl = await B.waitFor(isType("file_list_request"));
  assert(fl.payload.path === "/tmp", "远程文件 FileListRequest 路由到被控端");

  // 7) 懒推流——SetCapture A→B
  A.send({ type: "set_capture", session_id: sid, active: false });
  const sc = await B.waitFor(isType("set_capture"));
  assert(sc.payload.active === false, "懒推流 SetCapture(active=false) 路由到被控端");

  // 8) 审计持久化(HTTP /api/audit)——验证落 SQLite
  await new Promise((r) => setTimeout(r, 400)); // 等审计异步写入
  const lr = await fetch(BASE + "/api/login", {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ user: ADMIN_USER, pass: ADMIN_PASS }),
  });
  const { token } = await lr.json();
  assert(!!token, "管理端登录签发 JWT");
  const audit = await (await fetch(BASE + "/api/audit", { headers: { authorization: "Bearer " + token } })).json();
  const chatRow = audit.find((r) => r.type === "chat" && r.text === chatText);
  assert(chatRow && chatRow.actor_id === "ep-a", "审计落库:chat 行(全文 + actor=ep-a)");
  assert(audit.some((r) => r.type === "chat" && r.text === reply), "审计落库:双向第二条 chat 行");
  assert(audit.some((r) => r.type === "command"), "审计落库:command 行");
  assert(audit.some((r) => r.type === "file_transfer"), "审计落库:file_transfer 行");

  // 9) 终端注册可查
  const eps = await (await fetch(BASE + "/api/endpoints", { headers: { authorization: "Bearer " + token } })).json();
  assert(eps.some((e) => e.info.id === "ep-a") && eps.some((e) => e.info.id === "ep-b"), "两终端在线注册表可查");

  A.close(); B.close();
  console.log(`\n端到端测试全部通过:${passed} 项断言`);
}
main().then(() => process.exit(0)).catch((e) => { console.error("\n端到端测试失败:", e.message); process.exit(1); });
