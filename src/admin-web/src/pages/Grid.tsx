import { AppShell } from "@/components/shell/app-shell";
import { MonitorGrid } from "@/components/monitor/monitor-grid";
import { useStore } from "@/store";

export function Grid() {
  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  return (
    <AppShell title="批量监控" online={online} total={total}>
      <MonitorGrid />
    </AppShell>
  );
}
