import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import { FolderIcon } from "lucide-react"

import { WidgetEmpty, WidgetShell, cfgInt, cfgStr, type WidgetProps } from "./widget-shell"
import { api } from "@/lib/api"
import type { Category } from "@/lib/types"

/** 分类列表组件：取 /api/categories（含 published 文章计数），count 限制展示条数 */
export function CategoriesWidget({ widget }: WidgetProps) {
  const title = cfgStr(widget.config, "title", "分类") || "分类"
  const count = cfgInt(widget.config, "count", 0, 0, 100)
  const [categories, setCategories] = useState<Category[]>([])

  useEffect(() => {
    let cancelled = false
    api
      .categories()
      .then((d) => {
        if (!cancelled) setCategories(d)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const shown = count > 0 ? categories.slice(0, count) : categories

  return (
    <WidgetShell title={title} icon={<FolderIcon />}>
      {shown.length === 0 ? (
        <WidgetEmpty text="暂无分类" />
      ) : (
        <ul className="-mx-2 flex flex-col">
          {shown.map((c) => (
            <li key={c.id}>
              <Link
                to={`/categories/${encodeURIComponent(c.name)}`}
                className="flex items-center justify-between rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-accent hover:text-accent-foreground"
              >
                <span>{c.name}</span>
                <span className="text-xs text-muted-foreground">{c.post_count}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </WidgetShell>
  )
}
