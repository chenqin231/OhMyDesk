import { AppShell } from "@/components/shell/app-shell";
import { AuditLog } from "@/components/audit/audit-log";
import { useStore } from "@/store";

export function Audit() {
  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  return (
    <AppShell title="会话审计" online={online} total={total}>
      <AuditLog />
    </AppShell>
  );
}
