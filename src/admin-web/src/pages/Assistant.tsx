import { AppShell } from "@/components/shell/app-shell";
import { AssistantPanel } from "@/components/assistant/assistant-panel";
import { useStore } from "@/store";

export function Assistant() {
  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  return (
    <AppShell title="AI 助手" online={online} total={total}>
      <div className="h-full min-h-[600px]">
        <AssistantPanel />
      </div>
    </AppShell>
  );
}
