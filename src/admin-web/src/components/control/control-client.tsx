import { useState } from "react";

import { LaunchPanel } from "@/components/control/launch-panel";
import { ConnectingCard } from "@/components/control/connecting-card";
import { RemoteSession } from "@/components/control/remote-session";
import { AuthRequestDialog } from "@/components/control/auth-request-dialog";
import { RejectedCard } from "@/components/control/rejected-card";
import { useStore } from "@/store";

// 远程控制会话客户端：发起 → 协商连接中 → 控制中/拒绝 三态
// G-3 补：rejected 态展示拒连结果卡片
export function ControlClient() {
  const remotePhase = useStore((s) => s.remotePhase);
  const remoteRejectReason = useStore((s) => s.remoteRejectReason);
  const startRemote = useStore((s) => s.startRemote);
  const endRemote = useStore((s) => s.endRemote);
  const resetRemote = useStore((s) => s.resetRemote);

  const [mode, setMode] = useState<"a" | "b">("a");
  const [targetName, setTargetName] = useState("");
  const [authOpen, setAuthOpen] = useState(false);

  function handleLaunch(m: "a" | "b", name: string, password: string | null) {
    setMode(m);
    setTargetName(name);
    startRemote(m, name, password);
  }

  return (
    <div className="h-screen w-full overflow-hidden bg-background text-foreground">
      {remotePhase === "launch" && (
        <div className="h-full overflow-auto">
          <LaunchPanel
            onLaunch={handleLaunch}
            onPreviewAuth={() => setAuthOpen(true)}
          />
        </div>
      )}

      {remotePhase === "connecting" && (
        <ConnectingCard
          targetName={targetName}
          mode={mode}
          onCancel={resetRemote}
        />
      )}

      {/* G-3：拒连结果态 */}
      {remotePhase === "rejected" && (
        <RejectedCard reason={remoteRejectReason} onRetry={resetRemote} />
      )}

      {remotePhase === "connected" && (
        <RemoteSession targetName={targetName} mode={mode} onDisconnect={endRemote} />
      )}

      {/* 被控端视角：授权请求弹窗（可单独预览） */}
      <AuthRequestDialog open={authOpen} onOpenChange={setAuthOpen} />
    </div>
  );
}
