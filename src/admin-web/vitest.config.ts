import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// 复用 tsconfig 的 @ 路径别名；node 环境（纯逻辑单测，不渲染组件）。
export default defineConfig({
  test: {
    environment: "node",
    // 仅纳入本计划的纯逻辑单测（均在 src/lib 下）。
    // 排除既有的 src/components/control/remote-geometry.test.ts——它是 node:assert
    // 顶层断言脚本（无 describe/it 套件），由其原生 runner 执行，不走 vitest。
    include: ["src/lib/**/*.test.ts"],
  },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
});
