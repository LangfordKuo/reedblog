import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import { ArchiveIcon, FolderIcon, TagsIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { api } from "@/lib/api"
import type { ArchiveMonth, Category, Tag } from "@/lib/types"

function SidebarEmpty({ text }: { text: string }) {
  return <p className="text-sm text-muted-foreground">{text}</p>
}

/** 公开博客侧栏：标签 / 分类 / 归档 */
export function BlogSidebar() {
  const [tags, setTags] = useState<Tag[]>([])
  const [categories, setCategories] = useState<Category[]>([])
  const [archive, setArchive] = useState<ArchiveMonth[]>([])

  useEffect(() => {
    api.tags().then(setTags).catch(() => {})
    api.categories().then(setCategories).catch(() => {})
    api.archive().then(setArchive).catch(() => {})
  }, [])

  const years = [...new Set(archive.map((a) => a.year))].sort((a, b) => b - a)

  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <TagsIcon />
            标签
          </CardTitle>
        </CardHeader>
        <CardContent>
          {tags.length === 0 ? (
            <SidebarEmpty text="暂无标签" />
          ) : (
            <div className="flex flex-wrap gap-2">
              {tags.map((t) => (
                <Badge key={t.id} variant="secondary" asChild>
                  <Link to={`/tags/${encodeURIComponent(t.name)}`} className="hover:bg-secondary/70">
                    {t.name}
                    <span className="ml-0.5 opacity-60">{t.post_count}</span>
                  </Link>
                </Badge>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <FolderIcon />
            分类
          </CardTitle>
        </CardHeader>
        <CardContent>
          {categories.length === 0 ? (
            <SidebarEmpty text="暂无分类" />
          ) : (
            <ul className="-mx-2 flex flex-col">
              {categories.map((c) => (
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
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <ArchiveIcon />
            归档
          </CardTitle>
        </CardHeader>
        <CardContent>
          {archive.length === 0 ? (
            <SidebarEmpty text="暂无归档" />
          ) : (
            <div className="flex flex-col gap-4">
              {years.map((year) => (
                <div key={year} className="flex flex-col gap-2">
                  <div className="text-sm font-semibold">{year} 年</div>
                  <div className="flex flex-wrap gap-x-4 gap-y-1">
                    {archive
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
        </CardContent>
      </Card>
    </div>
  )
}
