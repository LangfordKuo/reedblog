import { useEffect, useState, type FormEvent } from "react"
import { useSearchParams } from "react-router-dom"
import { SearchIcon, SearchXIcon } from "lucide-react"

import { BlogSidebar } from "@/components/blog-sidebar"
import { Highlight } from "@/components/highlight"
import { Pagination } from "@/components/pagination"
import { PostCard } from "@/components/post-feed"
import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api, errorMessage } from "@/lib/api"
import type { Page, SearchResult } from "@/lib/types"

const PER_PAGE = 10
/** 与后端 split_search_terms 一致：空白切分、上限 8 个词条（多余忽略） */
const MAX_TERMS = 8

function splitTerms(q: string): string[] {
  return q
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, MAX_TERMS)
}

/** /search?q=…：全文搜索结果页（URL 携带查询词，可分享/刷新） */
export default function SearchPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const q = (searchParams.get("q") ?? "").trim()
  const page = Math.max(1, Number(searchParams.get("page")) || 1)
  const [input, setInput] = useState(q)
  const [data, setData] = useState<Page<SearchResult> | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // URL 变化（header 再次搜索、前进/后退）时同步输入框
  useEffect(() => setInput(q), [q])

  // 查询词变化时重置回第 1 页
  useEffect(() => {
    if (searchParams.has("page")) {
      const next = new URLSearchParams(searchParams)
      next.delete("page")
      setSearchParams(next, { replace: true })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q])

  // 有 q 才请求（q 为空后端会 400）
  useEffect(() => {
    if (!q) {
      setData(null)
      setError(null)
      setLoading(false)
      return
    }
    let cancelled = false
    setLoading(true)
    api
      .search({ q, page, per_page: PER_PAGE })
      .then((d) => {
        if (!cancelled) {
          setData(d)
          setError(null)
        }
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [q, page])

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const next = new URLSearchParams()
    const kw = input.trim()
    if (kw) next.set("q", kw)
    setSearchParams(next)
  }

  const goPage = (p: number) => {
    const next = new URLSearchParams(searchParams)
    if (p <= 1) next.delete("page")
    else next.set("page", String(p))
    setSearchParams(next)
  }

  const terms = splitTerms(q)

  return (
    <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_288px]">
      <div>
        <form onSubmit={submit} className="mb-6 flex gap-2">
          <Input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="搜索文章…"
            aria-label="搜索关键词"
            className="max-w-sm"
          />
          <Button type="submit" variant="secondary">
            <SearchIcon className="size-4" />
            搜索
          </Button>
        </form>

        {!q ? (
          <div className="py-16 text-center text-muted-foreground">
            <p className="text-sm">输入关键词开始搜索</p>
          </div>
        ) : loading ? (
          <BlockSpinner />
        ) : error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-center text-sm text-destructive">
            {error}
          </div>
        ) : data ? (
          <>
            <h1 className="mb-6 text-2xl font-bold tracking-tight">
              「{q}」的搜索结果（共 {data.total} 篇）
            </h1>
            {data.items.length === 0 ? (
              <div className="flex flex-col items-center gap-2 py-16 text-muted-foreground">
                <SearchXIcon className="size-8" />
                <p className="text-sm">没有找到相关文章，换个关键词试试吧</p>
              </div>
            ) : (
              <div className="flex flex-col gap-8">
                {data.items.map((r) => (
                  <PostCard
                    key={r.id}
                    post={r}
                    titleNode={<Highlight text={r.title} terms={terms} />}
                    excerptNode={
                      r.snippet ? <Highlight text={r.snippet} terms={terms} /> : undefined
                    }
                  />
                ))}
              </div>
            )}
            <Pagination
              className="mt-8"
              page={data.page}
              perPage={data.per_page}
              total={data.total}
              onChange={goPage}
            />
          </>
        ) : null}
      </div>
      <aside className="lg:sticky lg:top-24 lg:self-start">
        <BlogSidebar />
      </aside>
    </div>
  )
}
