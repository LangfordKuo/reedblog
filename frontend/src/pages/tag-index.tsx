import { useEffect, useState } from "react"
import { Link } from "react-router-dom"

import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { api, errorMessage } from "@/lib/api"
import { applyPageMeta } from "@/lib/meta"
import type { Tag } from "@/lib/types"

export default function TagIndexPage() {
  const [tags, setTags] = useState<Tag[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    applyPageMeta({ title: "标签", path: "/tags" })
    api
      .tags()
      .then(setTags)
      .catch((e) => setError(errorMessage(e)))
  }, [])

  return (
    <div className="mx-auto max-w-3xl">
      <h1 className="mb-6 text-2xl font-bold tracking-tight">全部标签</h1>
      {error ? (
        <p className="text-sm text-destructive">{error}</p>
      ) : tags === null ? (
        <BlockSpinner />
      ) : tags.length === 0 ? (
        <p className="py-12 text-center text-sm text-muted-foreground">暂无标签</p>
      ) : (
        <div className="flex flex-wrap gap-2">
          {tags.map((t) => (
            <Badge key={t.id} variant="secondary" className="px-3 py-1.5 text-sm" asChild>
              <Link to={`/tags/${encodeURIComponent(t.name)}`} className="hover:bg-secondary/70">
                {t.name}
                <span className="ml-1 text-xs opacity-60">{t.post_count}</span>
              </Link>
            </Badge>
          ))}
        </div>
      )}
    </div>
  )
}
