import { AppShell } from "@/components/shell/app-shell";
import { ControlClient } from "@/components/control/control-client";
import { useStore } from "@/store";

export function Remote() {
  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  return (
    <AppShell title="远程控制" online={online} total={total}>
      <ControlClient />
    </AppShell>
  );
}
