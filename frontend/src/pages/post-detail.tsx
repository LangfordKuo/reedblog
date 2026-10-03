import { useEffect, useRef, useState } from "react"
import { Link, useParams } from "react-router-dom"
import { ArrowLeftIcon, CalendarDaysIcon, EyeIcon, FolderIcon, MessageSquareIcon } from "lucide-react"

import { CommentSection } from "@/components/comment-section"
import { LikeButton } from "@/components/like-button"
import { Markdown } from "@/components/markdown"
import { PostNav } from "@/components/post-nav"
import { PostToc } from "@/components/post-toc"
import { ReadingProgress } from "@/components/reading-progress"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { WidgetRegion } from "@/components/widgets"
import { ApiError, api, errorMessage } from "@/lib/api"
import { useThemeSettings } from "@/lib/theme-settings"
import { extractTocHeadings, setTocHeadings, shouldRenderToc, useTocHeadings } from "@/lib/toc"
import { cn, formatDate } from "@/lib/utils"
import { useRegionWidgets } from "@/lib/widgets"
import type { PostDetail } from "@/lib/types"

type Status = "loading" | "ok" | "notfound" | "error"

export default function PostDetailPage() {
  const { slug = "" } = useParams()
  const [post, setPost] = useState<PostDetail | null>(null)
  const [status, setStatus] = useState<Status>("loading")
  const [error, setError] = useState("")
  /** 正文容器（TOC 标题提取范围——评论区的标题不参与） */
  const contentRef = useRef<HTMLDivElement>(null)
  const { layout } = useThemeSettings()
  const threeColumn = layout === "topbar-minimal-three-column"
  // 双列布局侧栏由页面自带（三列布局右栏由布局壳统一渲染，见 site-layout）
  const hasSidebarWidgets = useRegionWidgets("sidebar").length > 0
  const hasToc = shouldRenderToc(useTocHeadings())

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

  // TOC 标题提取：正文渲染提交后扫描 h2/h3（TOC_MAX_LEVEL 可配到 h4），
  // 无 id 标题按 slug+去重后缀写回 DOM；产物写入全局 store（三列布局壳的
  // 右栏同源消费）。切换文章/离开页面时清空。带 #hash 进入时定位到对应标题。
  useEffect(() => {
    if (status !== "ok" || !contentRef.current) {
      setTocHeadings([])
      return
    }
    setTocHeadings(extractTocHeadings(contentRef.current))
    // 带 #hash 直达时定位到对应标题（hash 可能是 slug 化的中文/含非法转义，解码容错）
    try {
      const hash = decodeURIComponent(window.location.hash.slice(1))
      if (hash) document.getElementById(hash)?.scrollIntoView()
    } catch {
      /* 非法百分号编码的 hash：忽略定位 */
    }
    return () => setTocHeadings([])
  }, [status, post])

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

  // 双列布局：有 TOC 或侧栏组件时展开两列（TOC 在右栏最上、widgets 之前）；
  // 三列布局右栏（含 TOC）由布局壳统一渲染，内容区维持单列；两者皆无保持居中单列
  const showAside = !threeColumn && (hasToc || hasSidebarWidgets)
  return (
    <div
      className={
        showAside ? "grid gap-8 lg:grid-cols-[minmax(0,1fr)_288px]" : "mx-auto max-w-3xl"
      }
    >
      {/* 阅读进度条（fixed，随滚动增长） */}
      <ReadingProgress />
      <div className="min-w-0 max-w-3xl">
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
              {/* 浏览量（契约「浏览量与点赞」条款；本次访问已计入） */}
              <span className="inline-flex items-center gap-1.5">
                <EyeIcon className="size-4" />
                {post.view_count} 次浏览
              </span>
            </div>
          </header>
          <Separator className="my-6" />
          {/* 正文容器：TOC 标题提取范围（react-markdown 渲染产物） */}
          <div ref={contentRef}>
            <Markdown>{post.content_md}</Markdown>
          </div>
          {/* 点赞按钮（乐观更新 + 弹跳动画；key 保证切换文章时重挂载取新状态） */}
          <div className="mt-10 flex justify-center">
            <LikeButton key={post.slug} slug={post.slug} initialLikes={post.likes} />
          </div>
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
        {/* 上一篇/下一篇（契约「文章上一篇/下一篇」条款；评论区之前，左=更早、右=更晚） */}
        <PostNav prev={post.prev_post} next={post.next_post} />
        <Separator className="my-10" />
        <CommentSection slug={post.slug} />
      </div>
      {showAside && (
        <aside
          className={cn(
            "flex flex-col gap-6 lg:sticky lg:top-24 lg:self-start",
            // 仅 TOC（无 widgets）时窄屏不出空壳（PostToc 自身 hidden lg:block）
            !hasSidebarWidgets && "hidden lg:flex",
          )}
        >
          <PostToc />
          <WidgetRegion region="sidebar" />
        </aside>
      )}
    </div>
  )
}
