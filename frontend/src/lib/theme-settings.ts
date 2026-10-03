import { useSyncExternalStore } from "react"

import { api } from "./api"
import type { ThemeSettingDecl, ThemeSettingValue } from "./types"

/**
 * 主题设置全局 store（扩展契约「主题设置项-前端应用约定」）：
 * - applyActiveTheme 拉到激活主题后调用 loadThemeSettings(slug) 填充；
 * - 生效值写入 :root 的 `--theme-setting-<key（_ 换 -）>` CSS 变量与
 *   `data-setting-*` 属性；layout 生效值写 `data-layout`（布局组件与 theme.css 可读取）；
 * - React 组件通过 useThemeSettings() 订阅（useSyncExternalStore），
 *   后台保存/切换主题后重新应用即时生效，无需刷新；
 * - 接口失败静默回退：清空设置变量，layout 回退 topbar-two-column。
 */

/** 前端内置消费的布局枚举（default 主题 layout 设置的两个合法值） */
export type SiteLayoutKind = "topbar-two-column" | "topbar-minimal-three-column"

export const DEFAULT_LAYOUT: SiteLayoutKind = "topbar-two-column"

export interface ThemeSettingsState {
  /** 设置所属主题 slug；null = 尚未加载/加载失败（回退态） */
  slug: string | null
  settings: ThemeSettingDecl[]
  values: Record<string, ThemeSettingValue>
  /** layout 设置的生效值（未声明/未知值回退双列） */
  layout: SiteLayoutKind
}

const FALLBACK: ThemeSettingsState = {
  slug: null,
  settings: [],
  values: {},
  layout: DEFAULT_LAYOUT,
}

let state: ThemeSettingsState = FALLBACK
const listeners = new Set<() => void>()

/** 上一次已应用的设置 key（切换主题/回退时清理对应 CSS 变量与 data 属性） */
let appliedKeys: string[] = []

const dashKey = (key: string) => key.replace(/_/g, "-")

function emit() {
  for (const l of listeners) l()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** 组件订阅入口：布局骨架 / PostFeed 侧栏开关 / wide_layout 等读取 */
export function useThemeSettings(): ThemeSettingsState {
  return useSyncExternalStore(subscribe, () => state)
}

/** layout 生效值 → 内置布局枚举（未声明或未知值一律回退双列） */
function toLayout(v: ThemeSettingValue | undefined): SiteLayoutKind {
  return v === "topbar-minimal-three-column" ? v : DEFAULT_LAYOUT
}

/** 清掉上一次应用的全部 --theme-setting-* 变量与 data-setting-* 属性 */
function clearAppliedDom() {
  const root = document.documentElement
  for (const key of appliedKeys) {
    root.style.removeProperty(`--theme-setting-${dashKey(key)}`)
    root.removeAttribute(`data-setting-${dashKey(key)}`)
  }
  appliedKeys = []
}

/**
 * 应用一份生效值（DOM 副作用 + store 更新）。
 * 后台设置面板保存成功后用 PUT 响应直接调用，前台立即热生效。
 */
export function applyThemeSettingValues(
  slug: string,
  settings: ThemeSettingDecl[],
  values: Record<string, ThemeSettingValue>,
): void {
  const root = document.documentElement
  clearAppliedDom()
  for (const [key, v] of Object.entries(values)) {
    const s = String(v)
    root.style.setProperty(`--theme-setting-${dashKey(key)}`, s)
    root.setAttribute(`data-setting-${dashKey(key)}`, s)
  }
  appliedKeys = Object.keys(values)
  const layout = toLayout(values.layout)
  root.setAttribute("data-layout", layout)
  state = { slug, settings, values, layout }
  emit()
}

/** 回退态：清空设置变量，layout 回双列（接口不可用时保证内置默认样式） */
function resetThemeSettingValues(): void {
  clearAppliedDom()
  document.documentElement.setAttribute("data-layout", DEFAULT_LAYOUT)
  state = FALLBACK
  emit()
}

/** 拉取指定主题的生效设置并应用；失败静默回退（不阻塞主题令牌管线） */
export async function loadThemeSettings(slug: string | null): Promise<void> {
  if (!slug) {
    resetThemeSettingValues()
    return
  }
  try {
    const d = await api.themeSettings(slug)
    applyThemeSettingValues(d.slug, d.settings, d.values)
  } catch {
    resetThemeSettingValues()
  }
}
