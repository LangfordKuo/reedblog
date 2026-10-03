import { fileURLToPath, URL } from "node:url"

import { defineConfig } from "vitest/config"

// 单元测试独立配置：不加载 react/tailwind 插件（不测组件渲染），
// 因此与 `npm run build` 使用的 vite.config.ts 完全解耦，互不影响。
export default defineConfig({
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    // jsdom：toc 用例需要在真实 DOM 语义下验证 KaTeX 副本剔除、
    // cloneNode / querySelectorAll / style.scrollMarginTop 等行为
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
  },
})
