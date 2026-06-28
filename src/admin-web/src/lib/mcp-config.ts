const DEFAULT_MCP_SERVER_PATH = "/data/code/OhMyDesk/src/mcp/dist/index.js";

export function getMcpApiBase(): string {
  return window.location.origin;
}

export function buildMcpConfig(apiBase: string, token: string | null): string {
  return JSON.stringify(
    {
      mcpServers: {
        ohmydesk: {
          command: "node",
          args: [DEFAULT_MCP_SERVER_PATH],
          env: {
            OHMYDESK_API_BASE: apiBase,
            ...(token ? { OHMYDESK_API_TOKEN: token } : {}),
          },
        },
      },
    },
    null,
    2,
  );
}
