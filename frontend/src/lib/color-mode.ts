/**
 * 前台颜色模式（纯前端）：三态偏好 light / dark / system（跟随 prefers-color-scheme），
 * 持久化在 localStorage（key: reedblog-color-mode），默认 system。
 *
 * 实现方式：在 document.documentElement 上加/去 `dark` class ——
 * index.css 的 `.dark` 令牌变体与主题系统（lib/theme.ts）注入的 `.dark{…!important}`
 * 都已就位，本模块只负责切换 class，不重复造令牌，也不改动 theme.ts。
 *
 * 防 FOUC：index.html <head> 的内联同步脚本已在首次渲染前按同一偏好应用过一次；
 * 本模块负责运行期切换，并在 system 态下监听系统偏好变化实时跟随。
 * dark class 是全局的：前台切暗色后进入 /admin，后台同样呈现暗色（预期行为）。
 */

export type ColorMode = "light" | "dark" | "system"

const STORAGE_KEY = "reedblog-color-mode"

function isColorMode(v: string | null): v is ColorMode {
  return v === "light" || v === "dark" || v === "system"
}

/** 读取当前偏好；localStorage 不可用（隐私模式等）或值非法时回退 system */
export function getColorMode(): ColorMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY)
    if (isColorMode(v)) return v
  } catch {
    // 静默回退 system
  }
  return "system"
}

/** 按偏好加/去 <html> 的 dark class（不写 localStorage） */
export function applyColorMode(mode: ColorMode = getColorMode()): void {
  const dark = mode === "dark" || (mode === "system" && systemPrefersDark())
  document.documentElement.classList.toggle("dark", dark)
  ensureSystemListener()
}

/** 持久化偏好并立即应用（用户在 header 切换时调用；点了就固化选择，不再回 system） */
export function setColorMode(mode: ColorMode): void {
  try {
    localStorage.setItem(STORAGE_KEY, mode)
  } catch {
    // 无法持久化：本次会话内仍然生效
  }
  applyColorMode(mode)
}

/** 当前是否渲染为暗色（header 切换按钮据此显示 sun/moon 图标） */
export function isDarkRendered(): boolean {
  return document.documentElement.classList.contains("dark")
}

function systemPrefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches
}

// system 偏好监听器（模块级单例，勿重复注册）：仅 system 态时跟随切换
let systemListenerAttached = false

function ensureSystemListener(): void {
  if (systemListenerAttached) return
  systemListenerAttached = true
  const mq = window.matchMedia("(prefers-color-scheme: dark)")
  mq.addEventListener("change", (e) => {
    if (getColorMode() === "system") {
      document.documentElement.classList.toggle("dark", e.matches)
    }
  })
}
