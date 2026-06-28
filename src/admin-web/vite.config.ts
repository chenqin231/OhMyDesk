import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // 静态产物输出到 dist/static（不用默认 assets/）：避免与 SPA 路由 /assets（终端资产页）
  // 撞名——否则服务端 nest_service("/assets") 会拦截 /assets 刷新请求返回 404。
  build: { assetsDir: "static" },
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
    },
  },
});
