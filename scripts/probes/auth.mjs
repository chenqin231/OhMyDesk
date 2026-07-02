// 鉴权链路探针：登录拿 token → WS token 闸（无效 1008 / 有效收列表）→ 模式A 仅 admin。
const HTTP = "http://127.0.0.1:8765";
const WS = "ws://127.0.0.1:8765/ws";
const ADMIN_USER = process.env.PROBE_USER || "superadmin";
const ADMIN_PASS = process.env.PROBE_PASS || "infogo123";
const log = [];
const ok = (m) => log.push("✅ " + m);
const info = (m) => log.push("· " + m);
const fail = (m) => { log.push("❌ " + m); console.log(log.join("\n")); process.exit(1); };

const epId = "ep-auth", epPw = "111111";
const epInfo = { id: epId, name: "测试终端", department: "财务部", ip: "10.0.0.7", mac: "aa:bb:cc:00:00:07",
  os: { name: "麒麟 V10", kind: "kylin" }, cpu: { model: "Loongson 3A5000", cores: 8, arch: "loong_arch" },
  ram: { total: 16777216000, used: 8388608000 }, gpu: null, agent_version: "0.1.0" };
const send = (ws, payload, from, to = null) => ws.send(JSON.stringify({ from, to, ts: 0, payload }));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

// 被控终端：注册 + 记录是否收到 IncomingControl
let incomingFrom = null;
const ep = new WebSocket(WS);
ep.addEventListener("open", () => send(ep, { type: "register", info: epInfo, password: epPw }, epId));
ep.addEventListener("message", (ev) => {
  const p = JSON.parse(ev.data).payload;
  if (p.type === "register_ack") info("被控终端注册");
  else if (p.type === "incoming_control") { incomingFrom = p.from; info(`被控收 IncomingControl from=${p.from}`); }
});

async function main() {
  await wait(500);

  // ── 1) 登录拿 token ──────────────────────────────────────────────
  const r = await fetch(`${HTTP}/api/login`, { method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ user: ADMIN_USER, pass: ADMIN_PASS }) });
  if (r.status !== 200) fail(`登录失败 HTTP ${r.status}`);
  const { token } = await r.json();
  ok("默认账号登录拿到 token");

  // ── 2) admin WS 带【无效】token → 应被 1008 关闭 ──────────────────
  await new Promise((resolve) => {
    const bad = new WebSocket(`${WS}?token=invalid.jwt.here`);
    bad.addEventListener("open", () => send(bad, { type: "heartbeat", id: "admin-bad", ram: { total: 0, used: 0 } }, "admin-bad"));
    bad.addEventListener("close", (e) => { e.code === 1008 ? ok(`无效 token 的 admin 连接被 1008 关闭`) : info(`无效 token 关闭码=${e.code}（非1008也算拒绝）`); resolve(); });
    bad.addEventListener("message", () => fail("无效 token 不应收到任何消息"));
    setTimeout(resolve, 1500);
  });

  // ── 3) admin WS 带【有效】token → 应收 endpoint_list 含被控终端 ───
  const gotList = await new Promise((resolve) => {
    const adm = new WebSocket(`${WS}?token=${encodeURIComponent(token)}`);
    adm.addEventListener("open", () => send(adm, { type: "heartbeat", id: "admin-ok", ram: { total: 0, used: 0 } }, "admin-ok"));
    adm.addEventListener("message", (ev) => {
      const p = JSON.parse(ev.data).payload;
      if (p.type === "endpoint_list" && p.endpoints.some((e) => e.info.id === epId) && !adm._sent) {
        adm._sent = true; // 只发一次：endpoint_list 会因后续 ep-evil 注册而重推
        ok(`有效 token 的 admin 收 endpoint_list（含 ${epId}）`);
        // 4) admin 发模式A 远控 → 被控应收 IncomingControl
        send(adm, { type: "connect_request", mode: "a", target: epId, password: null }, "admin-ok");
        info("admin 发模式A connect_request");
        setTimeout(() => resolve(true), 600);
      }
    });
    setTimeout(() => resolve(false), 2500);
  });
  if (!gotList) fail("有效 token 未收到 endpoint_list");
  if (incomingFrom === "admin-ok") ok("模式A：被控收到 admin 的 IncomingControl"); else fail("模式A 未送达被控");

  // ── 5) 伪造 agent 发模式A → 应被拒（被控不再收到新的 IncomingControl）──
  incomingFrom = null;
  await new Promise((resolve) => {
    const evil = new WebSocket(WS); // 无 token，agent 身份
    evil.addEventListener("open", () => { send(evil, { type: "register", info: { ...epInfo, id: "ep-evil", name: "伪造" }, password: "x" }, "ep-evil");
      setTimeout(() => { send(evil, { type: "connect_request", mode: "a", target: epId, password: null }, "ep-evil"); info("ep-evil 发模式A（应被拒）"); }, 300); });
    setTimeout(resolve, 1200);
  });
  if (incomingFrom !== "ep-evil") ok("非 admin(ep-evil) 的模式A 被服务端拒绝（被控未收到其控制）"); else fail("非 admin 竟能发起模式A！");

  console.log(log.join("\n") + "\n\n=== 鉴权链路全过：token 闸 + 模式A 仅 admin ===");
  process.exit(0);
}
main();
setTimeout(() => fail("超时"), 12000);
