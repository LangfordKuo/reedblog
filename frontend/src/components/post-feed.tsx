import { useEffect, useState, type ReactNode } from "react"
import { Link, useSearchParams } from "react-router-dom"
import { CalendarDaysIcon, FolderIcon, MessageSquareIcon, PinIcon } from "lucide-react"

import { Pagination } from "@/components/pagination"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { WidgetRegion } from "@/components/widgets"
import { api, errorMessage } from "@/lib/api"
import { useThemeSettings } from "@/lib/theme-settings"
import { formatDate } from "@/lib/utils"
import { useRegionWidgets } from "@/lib/widgets"
import type { Page, PostPublic } from "@/lib/types"

interface PostFeedProps {
  tag?: string
  category?: string
  year?: number
  month?: number
  heading?: string
}

/** 文章列表（含分页）+ 侧栏，首页 / 标签 / 分类 / 归档月份页共用。
 *  三列布局（topbar-minimal-three-column）下右栏由布局壳统一提供，这里不再渲染自带侧栏。 */
export function PostFeed({ tag, category, year, month, heading }: PostFeedProps) {
  const { layout } = useThemeSettings()
  const threeColumn = layout === "topbar-minimal-three-column"
  // 双列布局下侧栏由 WidgetRegion("sidebar") 提供；无启用组件时收缩为单列（不渲染空壳）
  const hasSidebar = useRegionWidgets("sidebar").length > 0
  const [searchParams, setSearchParams] = useSearchParams()
  const page = Math.max(1, Number(searchParams.get("page")) || 1)
  const [data, setData] = useState<Page<PostPublic> | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // 筛选条件变化时重置回第 1 页
  useEffect(() => {
    if (searchParams.has("page")) {
      const next = new URLSearchParams(searchParams)
      next.delete("page")
      setSearchParams(next, { replace: true })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tag, category, year, month])

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    api
      // 不传 per_page：由后端按站点设置的 per_page 默认分页（响应回显实际值）
      .posts({ page, tag, category, year, month })
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
  }, [page, tag, category, year, month])

  const goPage = (p: number) => {
    const next = new URLSearchParams(searchParams)
    if (p <= 1) next.delete("page")
    else next.set("page", String(p))
    setSearchParams(next)
  }

  return (
    <div
      className={
        threeColumn || !hasSidebar
          ? undefined
          : "grid gap-8 lg:grid-cols-[minmax(0,1fr)_288px]"
      }
    >
      <div>
        {heading && <h1 className="mb-6 text-2xl font-bold tracking-tight">{heading}</h1>}
        {loading ? (
          <BlockSpinner />
        ) : error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-center text-sm text-destructive">
            {error}
          </div>
        ) : data && data.items.length === 0 ? (
          <div className="py-16 text-center text-muted-foreground">
            <p className="text-sm">暂无文章</p>
          </div>
        ) : (
          <div className="flex flex-col gap-8">
            {data?.items.map((post) => <PostCard key={post.id} post={post} />)}
          </div>
        )}
        {!loading && !error && data && (
          <Pagination
            className="mt-8"
            page={data.page}
            perPage={data.per_page}
            total={data.total}
            onChange={goPage}
          />
        )}
      </div>
      {!threeColumn && hasSidebar && (
        <aside className="lg:sticky lg:top-24 lg:self-start">
          <WidgetRegion region="sidebar" />
        </aside>
      )}
    </div>
  )
}

interface PostCardProps {
  post: PostPublic
  /** 标题渲染覆盖（搜索页用来注入 <mark> 高亮）；缺省渲染 post.title */
  titleNode?: ReactNode
  /** 摘要渲染覆盖（搜索页用来显示高亮后的 snippet）；缺省渲染 post.excerpt */
  excerptNode?: ReactNode
}

/** 文章卡片（首页/列表页与搜索结果共用；标题与摘要可用 React 节点覆盖） */
export function PostCard({ post, titleNode, excerptNode }: PostCardProps) {
  const excerpt = excerptNode ?? post.excerpt
  return (
    <article className="flex flex-col gap-2">
      <h2 className="flex flex-wrap items-center gap-2 text-xl font-semibold tracking-tight">
        {/* 置顶徽章（契约「文章置顶与定时发布」条款：前台列表摘要卡对置顶文章显示） */}
        {post.is_sticky && (
          <Badge className="shrink-0 border-orange-200 bg-orange-50 text-xs font-medium text-orange-700">
            <PinIcon />
            置顶
          </Badge>
        )}
        <Link
          to={`/posts/${encodeURIComponent(post.slug)}`}
          className="underline-offset-4 hover:underline"
        >
          {titleNode ?? post.title}
        </Link>
      </h2>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span className="inline-flex items-center gap-1">
          <CalendarDaysIcon className="size-3.5" />
          {formatDate(post.published_at)}
        </span>
        {post.category && (
          <Link
            to={`/categories/${encodeURIComponent(post.category.name)}`}
            className="inline-flex items-center gap-1 transition-colors hover:text-foreground"
          >
            <FolderIcon className="size-3.5" />
            {post.category.name}
          </Link>
        )}
        <span className="inline-flex items-center gap-1">
          <MessageSquareIcon className="size-3.5" />
          {post.comment_count} 条评论
        </span>
        {post.tags.slice(0, 3).map((t) => (
          <Link key={t.id} to={`/tags/${encodeURIComponent(t.name)}`}>
            <Badge variant="outline" className="font-normal">
              {t.name}
            </Badge>
          </Link>
        ))}
      </div>
      {excerpt ? (
        <p className="line-clamp-3 text-sm text-muted-foreground">{excerpt}</p>
      ) : null}
      <div>
        <Link
          to={`/posts/${encodeURIComponent(post.slug)}`}
          // 主题设置 accent_color 的内置消费点：变量未设置时 var() 失效按 unset 继承原色
          className="text-[color:var(--theme-setting-accent-color)] text-sm font-medium underline-offset-4 hover:underline"
        >
          阅读全文 →
        </Link>
      </div>
    </article>
  )
}
