// I2 拒连双分支验证：① 模式B密码错 → Reject；② 被控主动拒绝 → Reject
const log = [];
const vId = "ep-victim2", vPw = "pw-correct";
const vInfo = { id: vId, name: "被控2", department: null, ip: "10.0.0.98", mac: "11:22:33:44:55:66",
  os: { name: "Ubuntu", kind: "other" }, cpu: { model: "t", cores: 4, arch: "x86_64" },
  ram: { total: 1000, used: 500 }, gpu: null, agent_version: "0.1.0" };
const send = (ws, p, from) => ws.send(JSON.stringify({ from, to: null, ts: 0, payload: p }));
let phase = 0, master;

const victim = new WebSocket("ws://127.0.0.1:8765/ws");
victim.addEventListener("open", () => send(victim, { type: "register", info: vInfo, password: vPw }, vId));
victim.addEventListener("message", (ev) => {
  const p = JSON.parse(ev.data).payload;
  if (p.type === "register_ack") { log.push("被控2注册"); startMaster(); }
  else if (p.type === "incoming_control") {
    log.push("被控2收 IncomingControl(正确密码已过) → 主动拒绝");
    send(victim, { type: "auth_result", session_id: p.session_id, ok: false, reason: "用户拒绝" }, vId);
  }
});
function startMaster() {
  master = new WebSocket("ws://127.0.0.1:8765/ws");
  master.addEventListener("open", () => { log.push("测试1: 主控发【错密码】ConnectRequest"); send(master, { type: "connect_request", mode: "b", target: vId, password: "WRONG" }, "ep-m2"); });
  master.addEventListener("message", (ev) => {
    const p = JSON.parse(ev.data).payload;
    if (p.type === "reject") {
      if (phase === 0) {
        log.push(`✅ 测试1: 收 Reject「${p.reason}」(密码错拒连)`);
        phase = 1;
        log.push("测试2: 主控发【正确密码】ConnectRequest");
        send(master, { type: "connect_request", mode: "b", target: vId, password: vPw }, "ep-m2");
      } else {
        log.push(`✅ 测试2: 收 Reject「${p.reason}」(被控主动拒绝)`);
        console.log(log.join("\n") + "\n\n=== 拒连双分支验证通过(密码错 + 主动拒绝)===");
        process.exit(0);
      }
    }
  });
}
setTimeout(() => { console.log(log.join("\n") + "\n❌ 超时"); process.exit(1); }, 6000);
