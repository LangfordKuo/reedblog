import { useSyncExternalStore } from "react"

import { api } from "./api"
import { DEFAULT_LAYOUT, useThemeSettings, type SiteLayoutKind } from "./theme-settings"
import type { WidgetPosition, WidgetPublic } from "./types"

/**
 * 主题组件（widgets）全局 store（契约「主题组件」）：
 * - applyActiveTheme 拉到激活主题后调用 loadWidgets(slug) 填充；
 * - 只缓存**生效配置**（公开端点已过滤 enabled、按 sort_order 排序、
 *   custom 组件 html 已完成 {{param}} 替换）；
 * - 布局降级映射（regionForPosition）：双列布局把 left/right 并入 sidebar；
 *   三列布局把 sidebar 映射到右栏——契约「主题组件-布局降级」；
 * - React 组件通过 useWidgets() 订阅；后台保存/切换主题后重新拉取即时生效；
 * - 接口失败静默回退空列表（各区域不渲染空壳）。
 */

export type WidgetRegion = "sidebar" | "left" | "right" | "footer"

export interface WidgetsState {
  /** 组件所属主题 slug；null = 尚未加载/加载失败（回退空） */
  slug: string | null
  widgets: WidgetPublic[]
}

const FALLBACK: WidgetsState = { slug: null, widgets: [] }

let state: WidgetsState = FALLBACK
const listeners = new Set<() => void>()

function emit() {
  for (const l of listeners) l()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** 组件订阅入口 */
export function useWidgets(): WidgetsState {
  return useSyncExternalStore(subscribe, () => state)
}

/** 拉取指定主题的生效组件配置并写入 store；失败静默回退空列表 */
export async function loadWidgets(slug: string | null): Promise<void> {
  if (!slug) {
    state = FALLBACK
    emit()
    return
  }
  try {
    const d = await api.themeWidgets(slug)
    state = { slug: d.slug, widgets: d.widgets ?? [] }
  } catch {
    state = FALLBACK
  }
  emit()
}

/** 某布局下可放置组件的区域（后台位置下拉的选项来源，契约「主题组件」） */
export function regionsForLayout(layout: SiteLayoutKind): WidgetRegion[] {
  return layout === "topbar-minimal-three-column"
    ? ["left", "right", "footer"]
    : ["sidebar", "footer"]
}

/**
 * 规范位置 → 当前布局的实际渲染区域（布局降级映射）：
 * - footer 恒映射 footer；
 * - 三列布局：left→left，right/sidebar→right（sidebar 降级到右栏）；
 * - 双列布局：sidebar/left/right→sidebar（left/right 并入侧栏）。
 */
export function regionForPosition(
  position: WidgetPosition,
  layout: SiteLayoutKind = DEFAULT_LAYOUT,
): WidgetRegion {
  if (position === "footer") return "footer"
  if (layout === "topbar-minimal-three-column") {
    return position === "left" ? "left" : "right"
  }
  return "sidebar"
}

/**
 * 取映射到某区域的组件（按 sort_order ASC；后端已排序，这里保持稳定过滤）。
 * 布局壳/页面据此渲染各区域；返回空数组时区域不渲染空壳。
 */
export function widgetsForRegion(
  widgets: WidgetPublic[],
  region: WidgetRegion,
  layout: SiteLayoutKind,
): WidgetPublic[] {
  return widgets.filter((w) => regionForPosition(w.position, layout) === region)
}

/**
 * 订阅某区域当前生效的组件列表（合并 widgets store 与当前 layout）。
 * 布局壳/页面用 length 判断是否渲染侧栏空壳（禁用全部组件时不渲染）。
 */
export function useRegionWidgets(region: WidgetRegion): WidgetPublic[] {
  const { widgets } = useWidgets()
  const { layout } = useThemeSettings()
  return widgetsForRegion(widgets, region, layout)
}

/** 位置的中文显示名（后台下拉/前台无障碍标签共用） */
export const POSITION_LABELS: Record<WidgetPosition, string> = {
  sidebar: "侧栏",
  left: "左栏",
  right: "右栏",
  footer: "页脚",
}

/** 区域的中文显示名（后台按当前布局展示可用区域） */
export const REGION_LABELS: Record<WidgetRegion, string> = {
  sidebar: "侧栏",
  left: "左栏",
  right: "右栏",
  footer: "页脚",
}
