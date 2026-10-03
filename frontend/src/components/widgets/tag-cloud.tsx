import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import { TagsIcon } from "lucide-react"

import { WidgetEmpty, WidgetShell, cfgInt, cfgStr, type WidgetProps } from "./widget-shell"
import { api } from "@/lib/api"
import type { Tag } from "@/lib/types"

/**
 * 标签云组件：取 /api/tags，字号按文章数在 [min,max] 区间线性加权。
 * count 参数限制展示的最大标签数（按文章数降序取前 N，再按名称稳定展示）。
 */
export function TagCloudWidget({ widget }: WidgetProps) {
  const title = cfgStr(widget.config, "title", "标签云") || "标签云"
  const count = cfgInt(widget.config, "count", 20, 1, 100)
  const [tags, setTags] = useState<Tag[]>([])

  useEffect(() => {
    let cancelled = false
    api
      .tags()
      .then((d) => {
        if (!cancelled) setTags(d)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  // 按文章数降序取前 count 个
  const shown = [...tags].sort((a, b) => b.post_count - a.post_count).slice(0, count)
  const maxCount = shown.reduce((m, t) => Math.max(m, t.post_count), 1)
  const minCount = shown.reduce((m, t) => Math.min(m, t.post_count), 0)
  const span = Math.max(1, maxCount - minCount)
  // 字号 0.75rem ~ 1.35rem 线性加权
  const fontSize = (c: number) => `${(0.75 + ((c - minCount) / span) * 0.6).toFixed(3)}rem`

  return (
    <WidgetShell title={title} icon={<TagsIcon />}>
      {shown.length === 0 ? (
        <WidgetEmpty text="暂无标签" />
      ) : (
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-2">
          {shown.map((t) => (
            <Link
              key={t.id}
              to={`/tags/${encodeURIComponent(t.name)}`}
              style={{ fontSize: fontSize(t.post_count) }}
              className="leading-none text-muted-foreground transition-colors hover:text-primary hover:underline"
              title={`${t.post_count} 篇文章`}
            >
              {t.name}
            </Link>
          ))}
        </div>
      )}
    </WidgetShell>
  )
}
