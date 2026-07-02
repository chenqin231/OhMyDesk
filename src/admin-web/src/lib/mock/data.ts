// Mock 数据 generator — 形状严格对齐 ts-rs 生成的 protocol 类型（snake_case + 嵌套）
// 禁止在此处使用 Date.now()，nowSec 由调用方注入

import type { EndpointView } from "@/lib/types/EndpointView";
import type { AuditLog } from "@/lib/types/AuditLog";
import type { Session } from "@/lib/types/Session";

const GB = BigInt(1024 * 1024 * 1024);

// 6 台终端，覆盖三方一致性要求的信创组合
// 字段 = protocol EndpointView（含 Wave0 补充的 department A-1）
export function makeEndpoints(nowSec: number): EndpointView[] {
  return [
    {
      info: {
        id: "ep-001",
        name: "张伟",
        department: "财务部",
        ip: "10.0.0.21",
        mac: "AA:BB:CC:00:00:21",
        os: { name: "银河麒麟 V10 SP3", kind: "kylin" },
        cpu: { model: "Loongson 3A5000", cores: 4, arch: "loong_arch" },
        ram: { total: BigInt(16) * GB, used: BigInt(6) * GB + BigInt(450 * 1024 * 1024) },
        gpu: { model: "JJM7200", vram: BigInt(4) * GB },
        agent_version: "0.1.0",
      },
      online: true,
      last_seen: BigInt(nowSec - 10),
      xinchuang: "信创·麒麟·龙芯",
    },
    {
      info: {
        id: "ep-002",
        name: "李娜",
        department: "研发中心",
        ip: "10.0.0.58",
        mac: "AA:BB:CC:00:00:58",
        os: { name: "统信 UOS 20 专业版", kind: "uos" },
        cpu: { model: "Kunpeng 920-4826", cores: 8, arch: "aarch64" },
        ram: { total: BigInt(32) * GB, used: BigInt(21) * GB + BigInt(400 * 1024 * 1024) },
        gpu: { model: "Ascend 310", vram: BigInt(8) * GB },
        agent_version: "0.1.0",
      },
      online: true,
      last_seen: BigInt(nowSec - 25),
      xinchuang: "信创·统信·鲲鹏",
    },
    {
      info: {
        id: "ep-003",
        name: "陈强",
        department: "销售一部",
        ip: "10.0.0.44",
        mac: "AA:BB:CC:00:00:44",
        os: { name: "Windows 11 23H2", kind: "windows" },
        cpu: { model: "Intel Core i7-12700", cores: 12, arch: "x86_64" },
        ram: { total: BigInt(16) * GB, used: BigInt(9) * GB + BigInt(800 * 1024 * 1024) },
        gpu: { model: "NVIDIA RTX 3060", vram: BigInt(12) * GB },
        agent_version: "0.1.0",
      },
      online: true,
      last_seen: BigInt(nowSec - 5),
      xinchuang: "非信创·Windows·x86_64",
    },
    {
      info: {
        id: "ep-004",
        name: "赵敏",
        department: "人力资源部",
        ip: "10.0.0.19",
        mac: "AA:BB:CC:00:00:19",
        os: { name: "银河麒麟 V10 SP3", kind: "kylin" },
        cpu: { model: "Kunpeng 920-3211", cores: 4, arch: "aarch64" },
        ram: { total: BigInt(16) * GB, used: BigInt(8) * GB },
        gpu: null,
        agent_version: "0.1.0",
      },
      online: true,
      last_seen: BigInt(nowSec - 80),
      xinchuang: "信创·麒麟·鲲鹏",
    },
    {
      info: {
        id: "ep-005",
        name: "王芳",
        department: "行政部",
        ip: "10.0.0.07",
        mac: "AA:BB:CC:00:00:07",
        os: { name: "统信 UOS 20 专业版", kind: "uos" },
        cpu: { model: "Loongson 3A6000", cores: 4, arch: "loong_arch" },
        ram: { total: BigInt(8) * GB, used: BigInt(0) },
        gpu: null,
        agent_version: "0.1.0",
      },
      online: false,
      last_seen: BigInt(nowSec - 3600 * 5),
      xinchuang: "信创·统信·龙芯",
    },
    {
      info: {
        id: "ep-006",
        name: "周婷",
        department: "品牌设计部",
        ip: "10.0.0.62",
        mac: "AA:BB:CC:00:00:62",
        os: { name: "Windows 11 23H2", kind: "windows" },
        cpu: { model: "AMD Ryzen 7 5800", cores: 8, arch: "x86_64" },
        ram: { total: BigInt(32) * GB, used: BigInt(27) * GB + BigInt(100 * 1024 * 1024) },
        gpu: { model: "NVIDIA RTX 3060", vram: BigInt(12) * GB },
        agent_version: "0.1.0",
      },
      online: false,
      last_seen: BigInt(nowSec - 3600 * 18),
      xinchuang: "非信创·Windows·x86_64",
    },
  ];
}

// 会话列表（~30 条审计事件，覆盖 connect/screenshot/input/disconnect/auth_fail/reject）
export function makeSessions(nowSec: number): Session[] {
  const d = (hoursAgo: number) => BigInt(nowSec - hoursAgo * 3600);
  // 真实 WEB 操作人身份（Task 8：审计展示真实登录账号）
  const op = (
    userId: string,
    username: string,
    role: string,
  ): Pick<Session, "operator_user_id" | "operator_username" | "operator_role"> => ({
    operator_user_id: userId,
    operator_username: username,
    operator_role: role,
  });
  // 旧数据：升级前的历史会话，无 WEB 身份 → 显示「旧版本记录」
  const legacy: Pick<Session, "operator_user_id" | "operator_username" | "operator_role"> = {
    operator_user_id: null,
    operator_username: null,
    operator_role: null,
  };
  return [
    { id: "ses-001", mode: "a", from_id: "admin-001", to_id: "ep-001", start_at: d(2), end_at: d(2) - BigInt(204), status: "ended", ...op("u-001", "张伟", "superadmin") },
    { id: "ses-002", mode: "b", from_id: "ep-003", to_id: "ep-002", start_at: d(4), end_at: d(4) - BigInt(341), status: "rejected", ...legacy },
    { id: "ses-003", mode: "a", from_id: "admin-001", to_id: "ep-004", start_at: d(6), end_at: d(6) - BigInt(535), status: "ended", ...op("u-002", "李强", "admin") },
    { id: "ses-004", mode: "b", from_id: "ep-002", to_id: "ep-001", start_at: d(8), end_at: null, status: "active", ...legacy },
    { id: "ses-005", mode: "a", from_id: "admin-001", to_id: "ep-003", start_at: d(24), end_at: d(24) - BigInt(728), status: "ended", ...op("u-001", "张伟", "superadmin") },
    { id: "ses-006", mode: "a", from_id: "admin-001", to_id: "ep-002", start_at: d(26), end_at: d(26) - BigInt(113), status: "rejected", ...op("u-003", "王芳", "admin") },
  ];
}

// 审计事件流（~30 条，覆盖多会话，type 取值 = spec C-1 集合）
export function makeAuditLogs(nowSec: number): AuditLog[] {
  const ts = (hoursAgo: number, offsetSec = 0) =>
    BigInt(nowSec - hoursAgo * 3600 - offsetSec);
  const identity: Pick<AuditLog, "actor_user_id" | "actor_username" | "actor_role"> = {
    actor_user_id: null,
    actor_username: null,
    actor_role: null,
  };

  return [
    // ses-001：连接 → 截图×2 → 输入 → 断开
    { id: "al-001", session_id: "ses-001", ts: ts(2, 0), actor_id: "admin-001", type: "connect", text: "管理员 → ep-001，建立连接", ...identity },
    { id: "al-002", session_id: "ses-001", ts: ts(2, 30), actor_id: "admin-001", type: "screenshot", text: "截图 1 张", ...identity },
    { id: "al-003", session_id: "ses-001", ts: ts(2, 90), actor_id: "admin-001", type: "input", text: "输入操作 47 次", ...identity },
    { id: "al-004", session_id: "ses-001", ts: ts(2, 150), actor_id: "admin-001", type: "screenshot", text: "截图 1 张", ...identity },
    { id: "al-005", session_id: "ses-001", ts: ts(2, 204), actor_id: "admin-001", type: "disconnect", text: "管理员主动断开，时长 03:24", ...identity },

    // ses-002：模式B密码错 → 拒连
    { id: "al-006", session_id: "ses-002", ts: ts(4, 0), actor_id: "ep-003", type: "connect", text: "ep-003 → ep-002，发起模式B连接", ...identity },
    { id: "al-007", session_id: "ses-002", ts: ts(4, 10), actor_id: "ep-003", type: "auth_fail", text: "密码错误（第1次）", ...identity },
    { id: "al-008", session_id: "ses-002", ts: ts(4, 25), actor_id: "ep-003", type: "auth_fail", text: "密码错误（第2次）", ...identity },
    { id: "al-009", session_id: "ses-002", ts: ts(4, 40), actor_id: "ep-003", type: "reject", text: "连续密码错误，连接被拒绝", ...identity },

    // ses-003：连接 → 截图×3 → 输入 → 断开
    { id: "al-010", session_id: "ses-003", ts: ts(6, 0), actor_id: "admin-001", type: "connect", text: "管理员 → ep-004，建立连接", ...identity },
    { id: "al-011", session_id: "ses-003", ts: ts(6, 60), actor_id: "admin-001", type: "screenshot", text: "截图 1 张", ...identity },
    { id: "al-012", session_id: "ses-003", ts: ts(6, 180), actor_id: "admin-001", type: "input", text: "输入操作 156 次", ...identity },
    { id: "al-013", session_id: "ses-003", ts: ts(6, 300), actor_id: "admin-001", type: "screenshot", text: "截图 1 张", ...identity },
    { id: "al-014", session_id: "ses-003", ts: ts(6, 420), actor_id: "admin-001", type: "screenshot", text: "截图 1 张", ...identity },
    { id: "al-015", session_id: "ses-003", ts: ts(6, 535), actor_id: "admin-001", type: "disconnect", text: "管理员主动断开，时长 08:55", ...identity },

    // ses-004：进行中（active）
    { id: "al-016", session_id: "ses-004", ts: ts(8, 0), actor_id: "ep-002", type: "connect", text: "ep-002 → ep-001，模式B建立连接", ...identity },
    { id: "al-017", session_id: "ses-004", ts: ts(8, 60), actor_id: "ep-002", type: "screenshot", text: "截图 1 张", ...identity },
    { id: "al-018", session_id: "ses-004", ts: ts(8, 120), actor_id: "ep-002", type: "input", text: "输入操作 22 次", ...identity },

    // ses-005：昨天成功
    { id: "al-019", session_id: "ses-005", ts: ts(24, 0), actor_id: "admin-001", type: "connect", text: "管理员 → ep-003，建立连接", ...identity },
    { id: "al-020", session_id: "ses-005", ts: ts(24, 200), actor_id: "admin-001", type: "screenshot", text: "截图 5 张", ...identity },
    { id: "al-021", session_id: "ses-005", ts: ts(24, 400), actor_id: "admin-001", type: "input", text: "输入操作 213 次", ...identity },
    { id: "al-022", session_id: "ses-005", ts: ts(24, 600), actor_id: "admin-001", type: "screenshot", text: "截图 2 张", ...identity },
    { id: "al-023", session_id: "ses-005", ts: ts(24, 728), actor_id: "admin-001", type: "disconnect", text: "管理员主动断开，时长 12:08", ...identity },

    // ses-006：终端用户拒绝授权
    { id: "al-024", session_id: "ses-006", ts: ts(26, 0), actor_id: "admin-001", type: "connect", text: "管理员 → ep-002，发起授权请求", ...identity },
    { id: "al-025", session_id: "ses-006", ts: ts(26, 18), actor_id: "ep-002", type: "reject", text: "李娜 点击「拒绝」，会话未建立", ...identity },
  ];
}

// mock 画面帧：canvas 绘制深色底 + 标识文字 + 移动方块，转 JPEG base64
export function makeMockFrameBase64(
  endpointId: string,
  seq: number,
  nowMs: number,
  w = 1280,
  h = 720,
): string {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;

  // 深色背景
  ctx.fillStyle = "#0e1117";
  ctx.fillRect(0, 0, w, h);

  // 网格线
  ctx.strokeStyle = "#21262d";
  ctx.lineWidth = 1;
  for (let x = 0; x < w; x += 80) {
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
  }
  for (let y = 0; y < h; y += 80) {
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
  }

  // 移动方块
  const blockSize = 60;
  const period = 3000;
  const t = nowMs % period;
  const bx = Math.round(((w - blockSize) * t) / period);
  const by = Math.round(h / 2 - blockSize / 2);
  ctx.fillStyle = "#2f81f7";
  ctx.fillRect(bx, by, blockSize, blockSize);

  // 标识文字
  ctx.fillStyle = "#e6edf3";
  ctx.font = "bold 28px monospace";
  ctx.textAlign = "center";
  ctx.fillText(`MOCK FRAME  ${endpointId}  seq:${seq}`, w / 2, h / 2 - 60);
  ctx.font = "18px monospace";
  ctx.fillStyle = "#8b949e";
  ctx.fillText(new Date(nowMs).toISOString(), w / 2, h / 2 - 20);

  return canvas.toDataURL("image/jpeg", 0.85).split(",")[1] ?? "";
}

// mock 截图（与帧共用逻辑，但不含 seq 滚动方块）
export function makeMockScreenshotBase64(endpointId: string, nowMs: number, w = 1920, h = 1080): string {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;

  ctx.fillStyle = "#161b22";
  ctx.fillRect(0, 0, w, h);

  ctx.fillStyle = "#2f81f7";
  ctx.fillRect(w / 2 - 200, h / 2 - 80, 400, 160);

  ctx.fillStyle = "#ffffff";
  ctx.font = "bold 36px monospace";
  ctx.textAlign = "center";
  ctx.fillText(`SCREENSHOT  ${endpointId}`, w / 2, h / 2 - 20);
  ctx.font = "22px monospace";
  ctx.fillStyle = "#8b949e";
  ctx.fillText(new Date(nowMs).toLocaleTimeString("zh-CN"), w / 2, h / 2 + 30);

  return canvas.toDataURL("image/jpeg", 0.85).split(",")[1] ?? "";
}
