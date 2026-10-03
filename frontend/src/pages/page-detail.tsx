import { useEffect, useState } from "react"
import { Link, useParams } from "react-router-dom"
import { ArrowLeftIcon, ExternalLinkIcon, Link2Icon } from "lucide-react"

import { CommentSection } from "@/components/comment-section"
import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Separator } from "@/components/ui/separator"
import { ApiError, api, errorMessage } from "@/lib/api"
import type { PageDetail } from "@/lib/types"

type Status = "loading" | "ok" | "notfound" | "error"

/**
 * 页面详情（前台路由 /pages/:slug，契约「页面」条款）：
 * - content_html 为后端渲染产物，直接注入（prose 排版，与文章正文一致）
 * - kind=message_board：正文下方挂留言表单（复用评论组件，target=page）
 * - kind=links：正文下方渲染友情链接卡片列表
 * - 停用/不存在的页面后端返回 404 → 渲染「页面不存在」
 */
export default function PageDetailPage() {
  const { slug = "" } = useParams()
  const [page, setPage] = useState<PageDetail | null>(null)
  const [status, setStatus] = useState<Status>("loading")
  const [error, setError] = useState("")

  useEffect(() => {
    let cancelled = false
    setStatus("loading")
    api
      .page(slug)
      .then((p) => {
        if (cancelled) return
        setPage(p)
        setStatus("ok")
        document.title = p.title
      })
      .catch((e) => {
        if (cancelled) return
        if (e instanceof ApiError && e.status === 404) {
          setStatus("notfound")
        } else {
          setError(errorMessage(e))
          setStatus("error")
        }
      })
    return () => {
      cancelled = true
    }
  }, [slug])

  if (status === "loading") return <BlockSpinner label="加载页面…" />

  if (status !== "ok" || !page) {
    return (
      <div className="flex flex-col items-center gap-4 py-24 text-center">
        <h1 className="text-xl font-semibold">
          {status === "notfound" ? "页面不存在或已停用" : error || "加载失败"}
        </h1>
        <Button variant="outline" asChild>
          <Link to="/">
            <ArrowLeftIcon />
            返回首页
          </Link>
        </Button>
      </div>
    )
  }

  return (
    <div className="mx-auto max-w-3xl">
      <article>
        <header>
          <h1 className="text-3xl font-bold tracking-tight">{page.title}</h1>
        </header>
        <Separator className="my-6" />
        {/* 后端渲染的 HTML（pulldown-cmark + 文章同款钩子管线），prose 排版 */}
        <div
          className="prose prose-neutral max-w-none"
          dangerouslySetInnerHTML={{ __html: page.content_html }}
        />
      </article>

      {/* 友情链接：链接卡片列表 */}
      {page.kind === "links" && (
        <section className="mt-10 flex flex-col gap-4">
          <h2 className="flex items-center gap-2 text-lg font-semibold">
            <Link2Icon className="size-5" />
            链接列表 ({page.links.length})
          </h2>
          {page.links.length === 0 ? (
            <p className="text-sm text-muted-foreground">暂无链接。</p>
          ) : (
            <div className="grid gap-3 sm:grid-cols-2">
              {page.links.map((l) => (
                <Card key={l.id} className="transition-colors hover:border-primary/40">
                  <CardContent className="flex flex-col gap-1 p-4">
                    <a
                      href={l.url}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex items-center gap-1.5 text-sm font-medium underline-offset-4 hover:underline"
                    >
                      {l.name}
                      <ExternalLinkIcon className="size-3.5 text-muted-foreground" />
                    </a>
                    {l.description && (
                      <p className="text-xs text-muted-foreground">{l.description}</p>
                    )}
                  </CardContent>
                </Card>
              ))}
            </div>
          )}
        </section>
      )}

      {/* 留言板：正文下方挂留言表单（复用评论管线组件） */}
      {page.kind === "message_board" && (
        <>
          <Separator className="my-10" />
          <CommentSection slug={page.slug} target="page" />
        </>
      )}
    </div>
  )
}
