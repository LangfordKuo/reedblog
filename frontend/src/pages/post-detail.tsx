import { useEffect, useState } from "react"
import { Link, useParams } from "react-router-dom"
import { ArrowLeftIcon, CalendarDaysIcon, FolderIcon, MessageSquareIcon } from "lucide-react"

import { CommentSection } from "@/components/comment-section"
import { Markdown } from "@/components/markdown"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { ApiError, api, errorMessage } from "@/lib/api"
import { formatDate } from "@/lib/utils"
import type { PostDetail } from "@/lib/types"

type Status = "loading" | "ok" | "notfound" | "error"

export default function PostDetailPage() {
  const { slug = "" } = useParams()
  const [post, setPost] = useState<PostDetail | null>(null)
  const [status, setStatus] = useState<Status>("loading")
  const [error, setError] = useState("")

  useEffect(() => {
    let cancelled = false
    setStatus("loading")
    api
      .post(slug)
      .then((p) => {
        if (cancelled) return
        setPost(p)
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

  if (status === "loading") return <BlockSpinner label="加载文章…" />

  if (status !== "ok" || !post) {
    return (
      <div className="flex flex-col items-center gap-4 py-24 text-center">
        <h1 className="text-xl font-semibold">
          {status === "notfound" ? "文章不存在或未发布" : error || "加载失败"}
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
        <header className="flex flex-col gap-3">
          <h1 className="text-3xl font-bold tracking-tight">{post.title}</h1>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-muted-foreground">
            <span className="inline-flex items-center gap-1.5">
              <CalendarDaysIcon className="size-4" />
              {formatDate(post.published_at)}
            </span>
            {post.category && (
              <Link
                to={`/categories/${encodeURIComponent(post.category.name)}`}
                className="inline-flex items-center gap-1.5 transition-colors hover:text-foreground"
              >
                <FolderIcon className="size-4" />
                {post.category.name}
              </Link>
            )}
            <span className="inline-flex items-center gap-1.5">
              <MessageSquareIcon className="size-4" />
              {post.comment_count} 条评论
            </span>
          </div>
        </header>
        <Separator className="my-6" />
        <Markdown>{post.content_md}</Markdown>
        {post.tags.length > 0 && (
          <footer className="mt-8 flex flex-wrap gap-2">
            {post.tags.map((t) => (
              <Badge key={t.id} variant="secondary" asChild>
                <Link to={`/tags/${encodeURIComponent(t.name)}`}># {t.name}</Link>
              </Badge>
            ))}
          </footer>
        )}
      </article>
      <Separator className="my-10" />
      <CommentSection slug={post.slug} />
    </div>
  )
}
