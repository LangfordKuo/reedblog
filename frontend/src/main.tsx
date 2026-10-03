import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { BrowserRouter } from "react-router-dom"

import "./index.css"
import App from "./App"
import { applyColorMode } from "./lib/color-mode"
import { applyActiveTheme } from "./lib/theme"

// 启动即应用颜色模式偏好（light/dark/system，与主题令牌互不依赖；
// index.html 的内联脚本已在首绘前应用过一次，这里兜底并挂 system 监听）。
applyColorMode()

// 启动即拉取激活主题并应用令牌/theme.css（公开站点与后台共用入口）。
// 该端点在未安装门禁白名单内，安装页同样能拿到 default 主题样式；
// 请求失败时静默回落 index.css 内置默认，不阻塞渲染。
void applyActiveTheme()

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </StrictMode>,
)
