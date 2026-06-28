// 切换：VITE_USE_MOCK=1 → mockTransport，否则 realTransport
import { mockTransport } from "./mock";
import { realTransport } from "./real";
import type { Transport } from "./types";

export const transport: Transport =
  import.meta.env.VITE_USE_MOCK === "1" ? mockTransport : realTransport;

export type { Transport, AuditQuery } from "./types";
