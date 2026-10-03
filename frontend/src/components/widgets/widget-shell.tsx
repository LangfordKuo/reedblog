import type { ReactNode } from "react"

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import type { WidgetConfigValues, WidgetPublic } from "@/lib/types"

/** 单个组件的渲染入参（从公开生效配置取数） */
export interface WidgetProps {
  widget: WidgetPublic
}

/** 组件卡片外壳：标题栏（有标题时）+ 内容区，统一各内置组件的外观 */
export function WidgetShell({
  title,
  icon,
  children,
}: {
  title?: string
  icon?: ReactNode
  children: ReactNode
}) {
  return (
    <Card>
      {title && (
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            {icon}
            {title}
          </CardTitle>
        </CardHeader>
      )}
      <CardContent className={title ? undefined : "pt-6"}>{children}</CardContent>
    </Card>
  )
}

export function WidgetEmpty({ text }: { text: string }) {
  return <p className="text-sm text-muted-foreground">{text}</p>
}

/** 读字符串参数（缺失/非字符串回退 fallback） */
export function cfgStr(config: WidgetConfigValues, key: string, fallback = ""): string {
  const v = config[key]
  return typeof v === "string" ? v : fallback
}

/** 读整数参数并钳制（缺失/非数字回退 fallback，再钳到 [min,max]） */
export function cfgInt(
  config: WidgetConfigValues,
  key: string,
  fallback: number,
  min = 1,
  max = 100,
): number {
  const v = config[key]
  const n = typeof v === "number" ? v : typeof v === "string" ? Number(v) : NaN
  const base = Number.isFinite(n) ? Math.trunc(n) : fallback
  return Math.min(max, Math.max(min, base))
}
