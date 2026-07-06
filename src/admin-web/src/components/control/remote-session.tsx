import { useEffect, useRef, useCallback, useState } from "react";
import { Camera, Download, Maximize, PhoneOff, TriangleAlert, Monitor, Terminal, FolderTree, MessageSquare } from "lucide-react";

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

/** 分段按钮组：三轴显示参数共用（样式与原「流畅/高清」一致） */
function SegGroup<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label?: string;
  options: { v: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div className="flex items-center gap-0.5">
      {label && (
        <span className="text-[10px] text-muted-foreground">{label}</span>
      )}
      <div
        aria-label={label}
        className="flex items-center overflow-hidden rounded-md border border-border"
      >
        {options.map((o) => (
          <button
            key={o.v}
            type="button"
            onClick={() => onChange(o.v)}
            className={`px-2.5 py-1 text-xs ${o.v === value ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-secondary"}`}
          >
            {o.label}
          </button>
        ))}
      </div>
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
  const containerRef = useRef<HTMLDivElement>(null);
  // 滚轮像素累加器(每会话一个,跨 wheel 事件保留余量)。见 makeRemoteScroll。
  const scrollAccRef = useRef(makeRemoteScroll());
  // 四标签：远程控制（画面）/ 命令行 / 文件传输 / 会话消息。仅「远程控制」标签转发键鼠到被控端。
  const [tab, setTab] = useState<ToolTab>("remote");
  const chatCount = useStore((s) => s.chatMessages.length);
  // 上次停留在「会话消息」标签时已读到的消息数；切到 chat 标签即清零未读。
  const [readChatCount, setReadChatCount] = useState(0);
  const unreadChat = tab === "chat" ? 0 : Math.max(0, chatCount - readChatCount);

  // 全屏：对远程画面容器请求浏览器全屏（再次点击退出）。
  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) {
      void document.exitFullscreen();
    } else {
      void containerRef.current?.requestFullscreen?.();
    }
  }, []);

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
  }, [toFrameCoords, sendInput, tab]);

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

  return (
    <div className="flex h-full min-h-[calc(100vh-7rem)] w-full flex-col bg-background">
      {/* 顶部细工具栏 */}
      <header className="flex h-12 shrink-0 items-center justify-between gap-4 border-b border-border bg-card px-4">
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
          {/* 三轴显示参数：分辨率 / 清晰度 / 帧率（仅远程控制标签） */}
          {tab === "remote" && (
            <div className="flex flex-wrap items-center gap-1.5">
              <SegGroup<ResolutionTier>
                label="分辨率"
                options={[
                  { v: "r720p", label: "720" },
                  { v: "r900p", label: "900" },
                  { v: "r1080p", label: "1080" },
                  { v: "native", label: "原生" },
                ]}
                value={remoteResolution}
                onChange={(v) => setRemoteDisplayParams({ resolution: v })}
              />
              <SegGroup<ClarityTier>
                label="清晰度"
                options={[
                  { v: "standard", label: "标准" },
                  { v: "high", label: "高清" },
                ]}
                value={remoteClarity}
                onChange={(v) => setRemoteDisplayParams({ clarity: v })}
              />
              <SegGroup<FpsTier>
                label="帧率"
                options={[
                  { v: "smooth", label: "流畅" },
                  { v: "standard", label: "标准" },
                  { v: "saver", label: "省流" },
                ]}
                value={remoteFps}
                onChange={(v) => setRemoteDisplayParams({ fps: v })}
              />
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
            className="relative flex h-full w-full max-w-[1920px] items-center justify-center overflow-hidden rounded-lg ring-1 ring-border cursor-pointer"
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
