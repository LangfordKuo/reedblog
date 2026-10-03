import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import { ArchiveIcon } from "lucide-react"

import { WidgetEmpty, WidgetShell, cfgInt, cfgStr, type WidgetProps } from "./widget-shell"
import { api } from "@/lib/api"
import type { ArchiveMonth } from "@/lib/types"

/** 归档组件：取 /api/archive（按月），按年分组展示；count 限制展示的月份总数 */
export function ArchiveWidget({ widget }: WidgetProps) {
  const title = cfgStr(widget.config, "title", "归档") || "归档"
  const count = cfgInt(widget.config, "count", 0, 0, 200)
  const [archive, setArchive] = useState<ArchiveMonth[]>([])

  useEffect(() => {
    let cancelled = false
    api
      .archive()
      .then((d) => {
        if (!cancelled) setArchive(d)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const shown = count > 0 ? archive.slice(0, count) : archive
  const years = [...new Set(shown.map((a) => a.year))].sort((a, b) => b - a)

  return (
    <WidgetShell title={title} icon={<ArchiveIcon />}>
      {shown.length === 0 ? (
        <WidgetEmpty text="暂无归档" />
      ) : (
        <div className="flex flex-col gap-3">
          {years.map((year) => (
            <div key={year} className="flex flex-col gap-1.5">
              <div className="text-sm font-semibold">{year} 年</div>
              <div className="flex flex-wrap gap-x-3 gap-y-1">
                {shown
                  .filter((a) => a.year === year)
                  .sort((a, b) => b.month - a.month)
                  .map((a) => (
                    <Link
                      key={`${a.year}-${a.month}`}
                      to={`/archive/${a.year}/${a.month}`}
                      className="text-sm text-muted-foreground transition-colors hover:text-foreground"
                    >
                      {a.month} 月
                      <span className="ml-1 text-xs">({a.count})</span>
                    </Link>
                  ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </WidgetShell>
  )
}
