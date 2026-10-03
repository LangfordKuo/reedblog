import type { ComponentType } from "react"

import { ArchiveWidget } from "./archive"
import { CategoriesWidget } from "./categories"
import { CustomHtmlWidget } from "./custom-html"
import { LinksWidget } from "./links"
import { HotPostsWidget, RecentPostsWidget } from "./post-list"
import { SiteInfoWidget } from "./site-info"
import { TagCloudWidget } from "./tag-cloud"
import type { WidgetProps } from "./widget-shell"
import { useRegionWidgets, type WidgetRegion as Region } from "@/lib/widgets"
import { cn } from "@/lib/utils"
import type { WidgetPublic } from "@/lib/types"

/**
 * 内置组件注册表：key → React 组件（与后端内置注册表 key 一一对应）。
 * kind=builtin 按 key 查表渲染；kind=custom（主题声明/后台自建）统一走 CustomHtmlWidget。
 * 未知 builtin key（后端新增而前端未实现）渲染为 null，不影响其余组件。
 */
const BUILTIN: Record<string, ComponentType<WidgetProps>> = {
  "recent-posts": RecentPostsWidget,
  "hot-posts": HotPostsWidget,
  "tag-cloud": TagCloudWidget,
  categories: CategoriesWidget,
  archive: ArchiveWidget,
  links: LinksWidget,
  "site-info": SiteInfoWidget,
}

/** 单个组件渲染分派 */
export function WidgetRenderer({ widget }: { widget: WidgetPublic }) {
  if (widget.kind === "custom") return <CustomHtmlWidget widget={widget} />
  const Cmp = BUILTIN[widget.key]
  if (!Cmp) return null
  return <Cmp widget={widget} />
}

/**
 * 布局区域渲染器：按当前 layout 把规范 position 降级映射到区域，
 * 渲染落入该区域的全部启用组件（后端已按 sort_order 排序）。
 * 区域内无组件时返回 null——不渲染空壳（契约「主题组件」）。
 * className 作用于外层容器（页脚区域用来做居中/限宽/留白）。
 */
export function WidgetRegion({ region, className }: { region: Region; className?: string }) {
  const list = useRegionWidgets(region)
  if (list.length === 0) return null
  return (
    <div className={cn("flex flex-col gap-6", className)}>
      {list.map((w) => (
        <WidgetRenderer key={w.key} widget={w} />
      ))}
    </div>
  )
}
