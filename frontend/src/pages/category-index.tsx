import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import { FolderIcon } from "lucide-react"

import { BlockSpinner } from "@/components/spinner"
import { api, errorMessage } from "@/lib/api"
import type { Category } from "@/lib/types"

export default function CategoryIndexPage() {
  const [categories, setCategories] = useState<Category[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    document.title = "分类"
    api
      .categories()
      .then(setCategories)
      .catch((e) => setError(errorMessage(e)))
  }, [])

  return (
    <div className="mx-auto max-w-3xl">
      <h1 className="mb-6 text-2xl font-bold tracking-tight">全部分类</h1>
      {error ? (
        <p className="text-sm text-destructive">{error}</p>
      ) : categories === null ? (
        <BlockSpinner />
      ) : categories.length === 0 ? (
        <p className="py-12 text-center text-sm text-muted-foreground">暂无分类</p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {categories.map((c) => (
            <Link
              key={c.id}
              to={`/categories/${encodeURIComponent(c.name)}`}
              className="flex items-center justify-between rounded-lg border bg-card p-4 transition-colors hover:border-primary/40"
            >
              <span className="inline-flex items-center gap-2 font-medium">
                <FolderIcon className="size-4 text-muted-foreground" />
                {c.name}
              </span>
              <span className="text-sm text-muted-foreground">{c.post_count} 篇</span>
            </Link>
          ))}
        </div>
      )}
    </div>
  )
}
