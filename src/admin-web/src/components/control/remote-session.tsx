import { useEffect, useRef, useCallback, useState } from "react";
import { Camera, Download, Maximize, PhoneOff, TriangleAlert, Monitor, Terminal, FolderTree, MessageSquare, SlidersHorizontal } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { frameSrc } from "@/lib/adapters/media";
import { useStore } from "@/store";
import type { InputEvent } from "@/lib/types/InputEvent";
import type { ResolutionTier } from "@/lib/types/ResolutionTier";
import type { ClarityTier } from "@/lib/types/ClarityTier";
import type { FpsTier } from "@/lib/types/FpsTier";
import { MODE_LABELS } from "@/components/control/launch-panel";
import {
  containedFrameRect,
  pointerToFrameCoords,
  remoteMouseButtonEvents,
  makeRemoteScroll,
  shouldBlockRemoteContextMenu,
} from "@/components/control/remote-geometry";
import { CommandPanel, FilePanel, ChatPanel, TabButton } from "@/components/control/remote-tools";
import { TouchGestureEngine, type GestureAction } from "@/lib/touch-gestures";

type ToolTab = "remote" | "cmd" | "file" | "chat";

type RemoteSessionProps = {
  targetName: string;
  mode: "a" | "b";
  onDisconnect: () => void;
};

// 顶部工具栏单项
function MetaItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-1.5 whitespace-nowrap">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="font-mono text-xs text-foreground">{value}</span>
    </div>
  );
}

// 三轴档位选项（模块级常量：RemoteSession 每帧随 remoteFrame 重渲染，避免逐帧新建数组）
const RES_OPTS: { v: ResolutionTier; label: string }[] = [
  { v: "r720p", label: "1280×720" },
  { v: "r900p", label: "1600×900" },
  { v: "r1080p", label: "1920×1080" },
  { v: "native", label: "原生" },
];
const CLARITY_OPTS: { v: ClarityTier; label: string }[] = [
  { v: "standard", label: "标准" },
  { v: "high", label: "高清" },
];
const FPS_OPTS: { v: FpsTier; label: string }[] = [
  { v: "smooth", label: "流畅" },
  { v: "standard", label: "标准" },
  { v: "saver", label: "省流" },
];

/** 标签 + 下拉：三轴显示参数共用（原生 select，深色主题描边） */
function LabeledSelect<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: { v: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <label className="flex items-center gap-1.5 whitespace-nowrap">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <select
        aria-label={label}
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
        className="rounded-md border border-border bg-secondary px-2 py-1 text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      >
        {options.map((o) => (
          <option key={o.v} value={o.v}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

// 光标同步叠加层：在主控「控制光标」位置（remoteCursorPos，帧坐标；桌面由鼠标、手机由触控引擎写入）
// 渲染被控端真实光标形状（箭头/文本 I 型/手型…），并隐藏本地系统光标 → 实现「看到被控端真实鼠标形状」。
// 位置直接改 DOM transform（不触发 <img> 重渲染），仅在 pos/frame 变化时重定位；形状 dataURL 变才换图。
function RemoteCursorOverlay({
  containerRef,
  active,
  touchMode,
}: {
  containerRef: React.RefObject<HTMLDivElement | null>;
  active: boolean;
  touchMode: boolean;
}) {
  const shape = useStore((s) => s.remoteCursorShape);
  const visible = useStore((s) => s.remoteCursorVisible);
  const pos = useStore((s) => s.remoteCursorPos);
  const frame = useStore((s) => s.remoteFrame);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = containerRef.current;
    const w = wrapRef.current;
    if (!el || !w || !pos || !frame) return;
    // 帧坐标 → 容器内显示 px：先算帧在容器里的实际显示矩形（object-contain letterbox），再按比例定位。
    const rect = el.getBoundingClientRect();
    const disp = containedFrameRect(
      { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
      frame,
    );
    const x = disp.left - rect.left + (pos.x / Math.max(1, frame.w)) * disp.width;
    const y = disp.top - rect.top + (pos.y / Math.max(1, frame.h)) * disp.height;
    w.style.transform = `translate(${x}px, ${y}px)`;
  }, [containerRef, pos, frame, shape, touchMode]);

  if (!active || !visible || !pos) return null;
  // 有被控端真实形状 → 渲染真实光标；否则手机触控模式下渲染兜底箭头，保证触控板有可见反馈
  //（被控端未升 0.6.0 不发形状、且 xcap 帧不含系统光标时，靠它让用户看到虚拟光标落点）。
  const showFallback = !shape && touchMode;
  if (!shape && !showFallback) return null;
  return (
    <div ref={wrapRef} style={{ position: "absolute", left: 0, top: 0, pointerEvents: "none", zIndex: 20 }}>
      {shape ? (
        <img
          src={shape.dataUrl}
          alt=""
          draggable={false}
          style={{
            display: "block",
            width: shape.w,
            height: shape.h,
            marginLeft: -shape.hotspotX,
            marginTop: -shape.hotspotY,
            imageRendering: "pixelated",
          }}
        />
      ) : (
        <svg width="22" height="22" viewBox="0 0 24 24" style={{ display: "block", filter: "drop-shadow(0 1px 1.5px rgba(0,0,0,0.6))" }}>
          <path d="M3 2 L3 19 L8 14.5 L11.3 21 L14 19.7 L10.8 13.4 L18 13 Z" fill="#fff" stroke="#000" strokeWidth="1.3" />
        </svg>
      )}
    </div>
  );
}

// G-1：canvas/img 消费远控帧；G-2：键鼠监听+坐标映射+发 Input 信封
// O-2 裁决：删除会话录制标记 UI
export function RemoteSession({ targetName, mode, onDisconnect }: RemoteSessionProps) {
  const remoteFrame = useStore((s) => s.remoteFrame);
  const remoteNotice = useStore((s) => s.remoteNotice);
  const remoteSessionId = useStore((s) => s.remoteSessionId);
  const sendEnvelope = useStore((s) => s.sendEnvelope);
  const remoteResolution = useStore((s) => s.remoteResolution);
  const remoteClarity = useStore((s) => s.remoteClarity);
  const remoteFps = useStore((s) => s.remoteFps);
  const setRemoteDisplayParams = useStore((s) => s.setRemoteDisplayParams);
  // 光标同步：有被控端形状时隐藏本地系统光标，改由 RemoteCursorOverlay 渲染真实光标。
  const remoteCursorShape = useStore((s) => s.remoteCursorShape);
  const setRemoteCursorPos = useStore((s) => s.setRemoteCursorPos);
  const containerRef = useRef<HTMLDivElement>(null);
  // 手机触控手势引擎（触控板模式）：跨 tick 持有虚拟光标与手势状态。
  const touchEngineRef = useRef<TouchGestureEngine | null>(null);
  // 滚轮像素累加器(每会话一个,跨 wheel 事件保留余量)。见 makeRemoteScroll。
  const scrollAccRef = useRef(makeRemoteScroll());
  // 四标签：远程控制（画面）/ 命令行 / 文件传输 / 会话消息。仅「远程控制」标签转发键鼠到被控端。
  const [tab, setTab] = useState<ToolTab>("remote");
  // 是否已发生过触摸操作 → 启用手机触控板兜底光标（区分桌面鼠标 vs 移动触控）。
  const [touchMode, setTouchMode] = useState(false);
  // CSS 伪全屏（iOS Safari 等不支持元素原生全屏时兜底：fixed 铺满视口）。
  const [cssFullscreen, setCssFullscreen] = useState(false);
  // 粗指针（移动端触屏）：据此切换移动端布局（画质控件收进菜单、加大触控目标）。
  const [coarse, setCoarse] = useState(false);
  // 移动端画质设置面板展开态（收进「画质」按钮，避免顶栏拥挤）。
  const [showQualityMenu, setShowQualityMenu] = useState(false);

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(pointer: coarse)");
    const on = () => setCoarse(mq.matches);
    on();
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  const chatCount = useStore((s) => s.chatMessages.length);
  // 上次停留在「会话消息」标签时已读到的消息数；切到 chat 标签即清零未读。
  const [readChatCount, setReadChatCount] = useState(0);
  const unreadChat = tab === "chat" ? 0 : Math.max(0, chatCount - readChatCount);

  // 全屏：对远程画面容器请求浏览器全屏（再次点击退出）。
  // 全屏：优先原生 Fullscreen API；移动端(尤其 iOS Safari)对非 video 元素不支持/被拒时，
  // 回退 CSS 伪全屏(整个远控页 fixed 铺满视口)——保证「全屏」按钮在所有移动浏览器都生效。
  const toggleFullscreen = useCallback(() => {
    const el = containerRef.current;
    if (document.fullscreenElement) {
      void document.exitFullscreen();
      return;
    }
    if (cssFullscreen) {
      setCssFullscreen(false);
      return;
    }
    const nativeOk =
      !!el && typeof el.requestFullscreen === "function" && !!document.fullscreenEnabled;
    if (nativeOk) {
      el!.requestFullscreen().catch(() => setCssFullscreen(true)); // 原生被拒 → CSS 兜底
    } else {
      setCssFullscreen(true);
    }
  }, [cssFullscreen]);

  // 截图：把当前帧（JPEG base64）触发浏览器下载为 .jpg。
  const saveScreenshot = useCallback(() => {
    if (!remoteFrame) return;
    const a = document.createElement("a");
    a.href = frameSrc({ data: remoteFrame.data });
    a.download = `${targetName}-${remoteFrame.seq}.jpg`;
    document.body.appendChild(a);
    a.click();
    a.remove();
  }, [remoteFrame, targetName]);

  const diagRing = useStore((s) => s.diagRing);
  // 下载诊断 JSON：仅标量指标，绝不含帧像素（脱敏）。
  const downloadDiag = useCallback(() => {
    const blob = new Blob([JSON.stringify({ target: targetName, exported_at: Date.now(), samples: diagRing }, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${targetName}-diag-${Date.now()}.json`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(a.href);
  }, [diagRing, targetName]);

  // G-2：坐标映射：将容器内鼠标坐标映射到帧分辨率
  const toFrameCoords = useCallback(
    (e: MouseEvent): { x: number; y: number } | null => {
      const el = containerRef.current;
      if (!el || !remoteFrame) return null;
      const rect = el.getBoundingClientRect();
      const displayRect = containedFrameRect(rect, remoteFrame);
      return pointerToFrameCoords(e, displayRect, remoteFrame);
    },
    [remoteFrame],
  );

  const sendInput = useCallback(
    (event: InputEvent) => {
      if (!remoteSessionId) return;
      sendEnvelope({ type: "input", session_id: remoteSessionId, event });
    },
    [remoteSessionId, sendEnvelope],
  );

  // G-2：监听键鼠事件并发 Input 信封（仅「远程控制」标签生效）
  useEffect(() => {
    const el = containerRef.current;
    if (!el || tab !== "remote") return;

    function onMouseMove(e: MouseEvent) {
      e.preventDefault();
      const coords = toFrameCoords(e);
      if (!coords) return;
      sendInput({ kind: "mouse_move", x: coords.x, y: coords.y });
      setRemoteCursorPos(coords); // 桌面：鼠标位置驱动光标叠加层
    }

    function onMouseDown(e: MouseEvent) {
      e.preventDefault();
      const coords = toFrameCoords(e);
      if (!coords) return;
      remoteMouseButtonEvents(coords, e.button, true).forEach(sendInput);
    }

    function onMouseUp(e: MouseEvent) {
      e.preventDefault();
      const coords = toFrameCoords(e);
      if (!coords) return;
      remoteMouseButtonEvents(coords, e.button, false).forEach(sendInput);
    }

    function onContextMenu(e: MouseEvent) {
      if (!shouldBlockRemoteContextMenu()) return;
      e.preventDefault();
    }

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const evt = scrollAccRef.current(e.deltaX, e.deltaY, e.deltaMode);
      if (evt) sendInput(evt);
    };

    el.addEventListener("mousemove", onMouseMove);
    el.addEventListener("mousedown", onMouseDown);
    el.addEventListener("mouseup", onMouseUp);
    el.addEventListener("contextmenu", onContextMenu);
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("mousemove", onMouseMove);
      el.removeEventListener("mousedown", onMouseDown);
      el.removeEventListener("mouseup", onMouseUp);
      el.removeEventListener("contextmenu", onContextMenu);
      el.removeEventListener("wheel", onWheel);
    };
  }, [toFrameCoords, sendInput, tab, setRemoteCursorPos]);

  // 手机触控手势（触控板模式，借鉴 UU 远程）：单指移=光标移、轻点=左键、长按=右键、
  // 双指移=滚动、双击后按住拖=左键拖拽。preventDefault 抑制浏览器缩放/滚动与合成鼠标事件。
  useEffect(() => {
    const el = containerRef.current;
    if (!el || tab !== "remote") return;

    const toPts = (tl: TouchList) => Array.from(tl).map((t) => ({ x: t.clientX, y: t.clientY }));

    const dispatch = (actions: GestureAction[]) => {
      for (const a of actions) {
        if (a.kind === "move") {
          sendInput({ kind: "mouse_move", x: a.x, y: a.y });
          setRemoteCursorPos({ x: a.x, y: a.y });
        } else if (a.kind === "button") {
          sendInput({ kind: "mouse_button", button: a.button, down: a.down });
        } else {
          sendInput({ kind: "scroll", dx: a.dx, dy: a.dy });
        }
      }
    };

    const ensureEngine = () => {
      const frame = useStore.getState().remoteFrame;
      const fw = frame?.w ?? 1280;
      const fh = frame?.h ?? 720;
      if (!touchEngineRef.current) {
        const start = useStore.getState().remoteCursorPos ?? { x: fw / 2, y: fh / 2 };
        touchEngineRef.current = new TouchGestureEngine(fw, fh, start);
      } else {
        touchEngineRef.current.setFrameSize(fw, fh);
      }
      return touchEngineRef.current;
    };

    let longTimer: number | undefined;
    const clearLong = () => {
      if (longTimer !== undefined) {
        clearTimeout(longTimer);
        longTimer = undefined;
      }
    };

    const onStart = (e: TouchEvent) => {
      e.preventDefault();
      setTouchMode(true); // 首次触摸即进入触控模式（启用兜底可见光标）
      const eng = ensureEngine();
      dispatch(eng.start(toPts(e.touches), Date.now()));
      // 单指按下即在虚拟光标处显示光标；启动长按计时（到点触发右键）。
      setRemoteCursorPos(eng.getCursor());
      clearLong();
      if (e.touches.length === 1) {
        longTimer = window.setTimeout(() => dispatch(eng.longPressFire()), 500);
      }
    };
    const onMove = (e: TouchEvent) => {
      e.preventDefault();
      const eng = touchEngineRef.current;
      if (!eng) return;
      const out = eng.move(toPts(e.touches), Date.now());
      if (out.length) clearLong(); // 已产生移动 → 取消长按
      dispatch(out);
    };
    const onEnd = (e: TouchEvent) => {
      e.preventDefault();
      clearLong();
      const eng = touchEngineRef.current;
      if (!eng) return;
      dispatch(eng.end(toPts(e.touches), Date.now()));
    };

    el.addEventListener("touchstart", onStart, { passive: false });
    el.addEventListener("touchmove", onMove, { passive: false });
    el.addEventListener("touchend", onEnd, { passive: false });
    el.addEventListener("touchcancel", onEnd, { passive: false });
    return () => {
      clearLong();
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
      el.removeEventListener("touchcancel", onEnd);
    };
  }, [tab, sendInput, setRemoteCursorPos]);

  // 键盘事件挂在 window（焦点无关）。
  // 用 e.key（已按 Shift/CapsLock 解析出大小写与上档符，如 "A"/"!"/"/"），而非 e.code（物理键
  // 位 "KeyA"/"Digit1"）——被控端注入侧对单字符直接走 Unicode，能正确还原所输入的字符；
  // 具名功能键（"Enter"/"Backspace"/"ArrowUp"/"Shift"…）由被控端映射为对应 enigo Key。
  // preventDefault 拦掉浏览器自身的按键行为（空格滚动、"/" 快速查找、F 键等），远控期间独占键盘。
  // 守卫：焦点在本地输入控件（命令行输入框/文件路径框等）时不拦截、不转发——否则用户无法打字
  //（Bug：全局键盘捕获会吞掉所有按键，导致底部「命令行」标签页输入框形同失效）。
  useEffect(() => {
    function isEditableTarget(): boolean {
      const el = document.activeElement;
      if (!el) return false;
      const tag = el.tagName;
      return (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        (el as HTMLElement).isContentEditable
      );
    }
    function onKeyDown(e: KeyboardEvent) {
      if (tab !== "remote" || isEditableTarget()) return;
      e.preventDefault();
      sendInput({ kind: "key", code: e.key, down: true });
    }
    function onKeyUp(e: KeyboardEvent) {
      if (tab !== "remote" || isEditableTarget()) return;
      e.preventDefault();
      sendInput({ kind: "key", code: e.key, down: false });
    }
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [sendInput, tab]);

  // 懒推流：仅「远程控制」标签需要桌面帧。进入该标签 → 恢复推流；离开 → 暂停（省内网带宽）。
  // 与 Slint 端一致，复用 set_capture 协议（计划①导出）。
  useEffect(() => {
    if (!remoteSessionId) return;
    sendEnvelope({ type: "set_capture", session_id: remoteSessionId, active: tab === "remote" });
  }, [tab, remoteSessionId, sendEnvelope]);

  // 三轴画质控件（桌面内联平铺 / 移动端收进「画质」按钮的下拉面板，复用同一份）。
  const qualityControls = (
    <>
      <LabeledSelect<ResolutionTier>
        label="分辨率"
        options={RES_OPTS}
        value={remoteResolution}
        onChange={(v) => setRemoteDisplayParams({ resolution: v })}
      />
      <LabeledSelect<ClarityTier>
        label="清晰度"
        options={CLARITY_OPTS}
        value={remoteClarity}
        onChange={(v) => setRemoteDisplayParams({ clarity: v })}
      />
      <LabeledSelect<FpsTier>
        label="帧率"
        options={FPS_OPTS}
        value={remoteFps}
        onChange={(v) => setRemoteDisplayParams({ fps: v })}
      />
    </>
  );

  return (
    <div
      className={`flex w-full flex-col bg-background ${
        cssFullscreen ? "fixed inset-0 z-50 h-screen" : "h-full min-h-[calc(100vh-7rem)]"
      }`}
    >
      {/* 顶部细工具栏：flex-wrap + min-h 而非固定 h-12——窄屏/移动端画质下拉+按钮换行显示，不再溢出裁掉。 */}
      <header className="flex min-h-12 shrink-0 flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-border bg-card px-4 py-1.5">
        {/* 左：远程控制中 + 目标 */}
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex items-center gap-1.5 rounded-full border border-online/30 bg-online/10 px-2.5 py-1">
            <span className="relative flex size-2">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-online opacity-60" />
              <span className="relative inline-flex size-2 rounded-full bg-online" />
            </span>
            <span className="text-xs font-medium text-online">远程控制中</span>
          </span>
          <span className="truncate text-sm font-medium text-foreground">{targetName}</span>
        </div>

        {/* 中：连接信息 */}
        <div className="hidden items-center gap-3 lg:flex">
          <MetaItem label="模式" value={MODE_LABELS[mode]} />
          {remoteFrame && (
            <>
              <Separator orientation="vertical" className="h-4" />
              <MetaItem label="分辨率" value={`${remoteFrame.w} × ${remoteFrame.h}`} />
              <Separator orientation="vertical" className="h-4" />
              <MetaItem label="帧序" value={String(remoteFrame.seq)} />
            </>
          )}
        </div>

        {/* 右：操作按钮 */}
        <div className="flex items-center gap-2">
          {/* 三轴显示参数：桌面平铺内联；移动端(粗指针)收进「画质」按钮的下拉面板，避免顶栏拥挤 */}
          {tab === "remote" && !coarse && (
            <div className="flex flex-wrap items-center gap-2.5">{qualityControls}</div>
          )}
          {tab === "remote" && coarse && (
            <div className="relative">
              <Button
                variant="outline"
                size="sm"
                aria-expanded={showQualityMenu}
                onClick={() => setShowQualityMenu((v) => !v)}
              >
                <SlidersHorizontal data-icon="inline-start" />
                画质
              </Button>
              {showQualityMenu && (
                <div className="absolute right-0 top-full z-40 mt-1.5 flex flex-col gap-3 rounded-lg border border-border bg-card p-3 shadow-xl">
                  {qualityControls}
                </div>
              )}
            </div>
          )}
          {tab === "remote" && (
            <>
              <Button variant="outline" size="sm" onClick={toggleFullscreen}>
                <Maximize data-icon="inline-start" />
                <span className="hidden sm:inline">全屏</span>
              </Button>
              <Button variant="outline" size="sm" onClick={saveScreenshot} disabled={!remoteFrame}>
                <Camera data-icon="inline-start" />
                <span className="hidden sm:inline">截图</span>
              </Button>
              <Button variant="outline" size="sm" onClick={downloadDiag} disabled={diagRing.length === 0}>
                <Download data-icon="inline-start" />
                <span className="hidden sm:inline">诊断</span>
              </Button>
            </>
          )}
          <Button
            size="sm"
            onClick={onDisconnect}
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            <PhoneOff data-icon="inline-start" />
            断开连接
          </Button>
        </div>
      </header>

      {/* 四标签栏：远程控制 / 命令行 / 文件传输 / 会话消息（连接态才渲染整个 RemoteSession，天然门控） */}
      <nav className="flex h-10 shrink-0 items-center gap-1 border-b border-border bg-card px-2">
        <TabButton active={tab === "remote"} onClick={() => setTab("remote")} icon={<Monitor className="size-3.5" />}>
          远程控制
        </TabButton>
        <TabButton active={tab === "cmd"} onClick={() => setTab("cmd")} icon={<Terminal className="size-3.5" />}>
          命令行
        </TabButton>
        <TabButton active={tab === "file"} onClick={() => setTab("file")} icon={<FolderTree className="size-3.5" />}>
          文件传输
        </TabButton>
        <TabButton
          active={tab === "chat"}
          onClick={() => { setTab("chat"); setReadChatCount(chatCount); }}
          icon={<MessageSquare className="size-3.5" />}
        >
          <span className="flex items-center gap-1.5">
            会话消息
            {unreadChat > 0 && (
              <span className="flex min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-[10px] font-medium leading-4 text-destructive-foreground">
                {unreadChat}
              </span>
            )}
          </span>
        </TabButton>
      </nav>

      {/* 主体：按标签切换。命令行/文件传输与远程画面平级（不再是底部停靠面板）。 */}
      <div className="flex min-h-0 flex-1 flex-col">
        {/* 远程控制标签：始终挂载（隐藏而非卸载），保持 containerRef 与第一帧不丢；非 remote 标签时 hidden。 */}
        <main
          className={`relative min-h-0 flex-1 items-center justify-center overflow-hidden bg-black p-3 md:p-6 ${tab === "remote" ? "flex" : "hidden"}`}
        >
          <div
            ref={containerRef}
            // touch-none(touch-action:none):关键——否则移动端浏览器把触摸当页面滚动/缩放吃掉，
            // 自定义手势收不到 touchmove。select-none 防长按选中文本。
            className={`relative flex h-full w-full max-w-[1920px] touch-none select-none items-center justify-center overflow-hidden rounded-lg ring-1 ring-border ${remoteCursorShape ? "cursor-none" : "cursor-pointer"}`}
          >
            {remoteFrame ? (
              <img
                src={frameSrc({ data: remoteFrame.data })}
                alt={`${targetName} 的远程桌面画面`}
                className="max-h-full max-w-full object-contain"
                draggable={false}
              />
            ) : remoteNotice ? (
              <div className="absolute inset-0 flex items-center justify-center bg-secondary px-8">
                <div className="flex max-w-md items-start gap-3 rounded-lg border border-warning/40 bg-warning/10 px-4 py-3 text-sm text-warning">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
                  <span className="leading-relaxed">{remoteNotice}</span>
                </div>
              </div>
            ) : (
              <div className="absolute inset-0 flex items-center justify-center bg-secondary text-sm text-muted-foreground">
                等待第一帧…
              </div>
            )}

            {/* 光标同步：被控端真实光标形状叠加层（在本地指针位置渲染，隐藏系统光标）；
                手机触控模式下无被控形状时渲染兜底箭头，保证触控板可见反馈。 */}
            <RemoteCursorOverlay containerRef={containerRef} active={tab === "remote"} touchMode={touchMode} />

            {/* 左上角常驻安全提示条 */}
            <div className="absolute left-3 top-3 flex items-center gap-2 rounded-md bg-warning/90 px-3 py-1.5 text-xs font-medium text-warning-foreground shadow-lg backdrop-blur-sm">
              <TriangleAlert className="size-3.5 shrink-0" aria-hidden />
              此终端正在被 管理员 远程协助
            </div>
          </div>
        </main>

        {tab === "cmd" && (
          <div className="min-h-0 flex-1">
            <CommandPanel />
          </div>
        )}
        {tab === "file" && (
          <div className="min-h-0 flex-1">
            <FilePanel />
          </div>
        )}
        {tab === "chat" && (
          <div className="min-h-0 flex-1">
            <ChatPanel />
          </div>
        )}
      </div>
    </div>
  );
}
