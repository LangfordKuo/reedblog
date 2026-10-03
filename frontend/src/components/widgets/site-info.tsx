import { useEffect, useState } from "react"
import { InfoIcon } from "lucide-react"

import { WidgetShell, cfgStr, type WidgetProps } from "./widget-shell"
import { api } from "@/lib/api"
import type { SiteSettings, SiteStats } from "@/lib/types"

/** 安装时间 → 运行天数（向上取整到整天，至少 1 天）；无效时间返回 null */
function runningDays(installedAt: string): number | null {
  if (!installedAt) return null
  const t = new Date(installedAt).getTime()
  if (Number.isNaN(t)) return null
  const days = Math.ceil((Date.now() - t) / 86400000)
  return days < 1 ? 1 : days
}

/**
 * 站点信息卡组件：站点名/描述（/api/site/settings）+ 文章数/评论数/运行天数
 * （/api/site/stats）。title 参数可覆盖卡片标题（默认「站点信息」，留空则不显示标题栏）。
 */
export function SiteInfoWidget({ widget }: WidgetProps) {
  const title = cfgStr(widget.config, "title", "站点信息")
  const [settings, setSettings] = useState<SiteSettings | null>(null)
  const [stats, setStats] = useState<SiteStats | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .siteSettings()
      .then((s) => {
        if (!cancelled) setSettings(s)
      })
      .catch(() => {})
    api
      .siteStats()
      .then((s) => {
        if (!cancelled) setStats(s)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const days = stats ? runningDays(stats.installed_at) : null
  const rows: [string, string | number][] = []
  if (stats) {
    rows.push(["文章", stats.post_count])
    rows.push(["评论", stats.comment_count])
  }
  if (days !== null) rows.push(["运行天数", `${days} 天`])

  return (
    <WidgetShell title={title || undefined} icon={<InfoIcon />}>
      <div className="flex flex-col gap-3">
        <div>
          <div className="text-sm font-semibold">{settings?.title || "reedblog"}</div>
          {(settings?.description || settings?.subtitle) && (
            <p className="mt-0.5 text-xs text-muted-foreground">
              {settings?.description || settings?.subtitle}
            </p>
          )}
        </div>
        {rows.length > 0 && (
          <dl className="grid grid-cols-3 gap-2 text-center">
            {rows.map(([label, value]) => (
              <div key={label} className="rounded-md bg-muted/50 px-1 py-2">
                <dd className="text-sm font-semibold">{value}</dd>
                <dt className="text-[11px] text-muted-foreground">{label}</dt>
              </div>
            ))}
          </dl>
        )}
      </div>
    </WidgetShell>
  )
}
