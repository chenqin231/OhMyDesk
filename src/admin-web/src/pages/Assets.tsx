import { AppShell } from "@/components/shell/app-shell";
import { TerminalAssets } from "@/components/assets/terminal-assets";
import { useStore } from "@/store";

export function Assets() {
  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  return (
    <AppShell title="终端资产" online={online} total={total}>
      <TerminalAssets />
    </AppShell>
  );
}
