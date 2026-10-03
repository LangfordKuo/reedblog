import { fileURLToPath, URL } from "node:url"

import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://localhost:3000",
        changeOrigin: true,
      },
    },
  },
  build: {
    rolldownOptions: {
      output: {
        // 路由级 lazy（src/App.tsx）之后，Markdown 渲染栈（react-markdown + KaTeX +
        // highlight.js）已经是只在文章详情/编辑器加载的异步 chunk，但仍超过 500 kB
        // 告警线。这里把体积最大的两个第三方渲染器再拆成独立 chunk：同一页面同时
        // 需要它们，字节总量不变（浏览器并行加载），只是每个 chunk 都不再触发告警，
        // 且便于单独长缓存（升级其一不必让另一个失效）。
        codeSplitting: {
          groups: [
            { name: "vendor-katex", test: /[\\/]node_modules[\\/]katex[\\/]/ },
            {
              name: "vendor-highlight",
              test: /[\\/]node_modules[\\/](highlight\.js|lowlight)[\\/]/,
            },
          ],
        },
      },
    },
  },
})
