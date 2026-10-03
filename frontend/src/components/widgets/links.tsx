import { useEffect, useState } from "react"
import { Link2Icon } from "lucide-react"

import { WidgetEmpty, WidgetShell, cfgInt, cfgStr, type WidgetProps } from "./widget-shell"
import { api } from "@/lib/api"
import type { PageLink } from "@/lib/types"

/**
 * 友情链接组件：读 links 页数据——先 /api/pages 找到 kind=links 的启用页，
 * 再 /api/pages/:slug 取其 links（sort_order ASC）；count 限制展示条数。
 */
export function LinksWidget({ widget }: WidgetProps) {
  const title = cfgStr(widget.config, "title", "友情链接") || "友情链接"
  const count = cfgInt(widget.config, "count", 0, 0, 100)
  const [links, setLinks] = useState<PageLink[]>([])

  useEffect(() => {
    let cancelled = false
    api
      .pages()
      .then((pages) => {
        const linksPage = pages.find((p) => p.kind === "links")
        if (!linksPage) return
        return api.page(linksPage.slug).then((d) => {
          if (!cancelled) setLinks(d.links ?? [])
        })
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const shown = count > 0 ? links.slice(0, count) : links

  return (
    <WidgetShell title={title} icon={<Link2Icon />}>
      {shown.length === 0 ? (
        <WidgetEmpty text="暂无链接" />
      ) : (
        <ul className="flex flex-col gap-2">
          {shown.map((l) => (
            <li key={l.id} className="flex flex-col">
              <a
                href={l.url}
                target="_blank"
                rel="noreferrer"
                className="text-sm transition-colors hover:text-primary hover:underline"
              >
                {l.name}
              </a>
              {l.description && (
                <span className="line-clamp-1 text-xs text-muted-foreground">
                  {l.description}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </WidgetShell>
  )
}
