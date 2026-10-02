import { useEffect, useMemo, useState } from "react"
import { Link } from "react-router-dom"

import { BlockSpinner } from "@/components/spinner"
import { api, errorMessage } from "@/lib/api"
import type { ArchiveMonth } from "@/lib/types"

export default function ArchivePage() {
  const [months, setMonths] = useState<ArchiveMonth[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    document.title = "归档"
    api
      .archive()
      .then(setMonths)
      .catch((e) => setError(errorMessage(e)))
  }, [])

  const byYear = useMemo(() => {
    const map = new Map<number, ArchiveMonth[]>()
    for (const m of months ?? []) {
      const arr = map.get(m.year) ?? []
      arr.push(m)
      map.set(m.year, arr)
    }
    return [...map.entries()].sort((a, b) => b[0] - a[0])
  }, [months])

  const totalCount = (months ?? []).reduce((sum, m) => sum + m.count, 0)

  return (
    <div className="mx-auto max-w-3xl">
      <h1 className="mb-6 text-2xl font-bold tracking-tight">
        归档
        {months && months.length > 0 && (
          <span className="ml-2 text-sm font-normal text-muted-foreground">
            共 {totalCount} 篇文章
          </span>
        )}
      </h1>
      {error ? (
        <p className="text-sm text-destructive">{error}</p>
      ) : months === null ? (
        <BlockSpinner />
      ) : months.length === 0 ? (
        <p className="py-12 text-center text-sm text-muted-foreground">暂无文章</p>
      ) : (
        <div className="flex flex-col gap-8">
          {byYear.map(([year, items]) => (
            <section key={year}>
              <h2 className="mb-3 text-lg font-semibold">{year} 年</h2>
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {items
                  .slice()
                  .sort((a, b) => b.month - a.month)
                  .map((m) => (
                    <Link
                      key={`${m.year}-${m.month}`}
                      to={`/archive/${m.year}/${m.month}`}
                      className="flex items-center justify-between rounded-lg border bg-card p-4 transition-colors hover:border-primary/40"
                    >
                      <span className="font-medium">{m.month} 月</span>
                      <span className="text-sm text-muted-foreground">{m.count} 篇</span>
                    </Link>
                  ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  )
}
