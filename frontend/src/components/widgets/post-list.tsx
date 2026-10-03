import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import { EyeIcon, FlameIcon, FileTextIcon, MessageSquareIcon } from "lucide-react"

import { WidgetEmpty, WidgetShell, cfgInt, cfgStr, type WidgetProps } from "./widget-shell"
import { api } from "@/lib/api"
import { formatDate } from "@/lib/utils"
import type { PostPublic } from "@/lib/types"

/**
 * 文章列表组件的共用实现：最新文章（order=recent）与热门文章（order=hot）。
 * 取数用公开 /api/posts（含 count 条数、title 标题参数）。
 */
function PostListWidget({
  widget,
  order,
  defaultTitle,
  icon,
  showCommentCount,
  showViewCount,
}: WidgetProps & {
  order: "recent" | "hot"
  defaultTitle: string
  icon: React.ReactNode
  showCommentCount?: boolean
  /** 副标题显示浏览数（契约「浏览量与点赞」：hot-posts 组件消费点） */
  showViewCount?: boolean
}) {
  const title = cfgStr(widget.config, "title", defaultTitle) || defaultTitle
  const count = cfgInt(widget.config, "count", 5, 1, 20)
  const [posts, setPosts] = useState<PostPublic[]>([])

  useEffect(() => {
    let cancelled = false
    api
      .posts({ order, per_page: count })
      .then((d) => {
        if (!cancelled) setPosts(d.items)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [order, count])

  return (
    <WidgetShell title={title} icon={icon}>
      {posts.length === 0 ? (
        <WidgetEmpty text="暂无文章" />
      ) : (
        <ul className="flex flex-col gap-2.5">
          {posts.map((p) => (
            <li key={p.id} className="flex flex-col gap-0.5">
              <Link
                to={`/posts/${encodeURIComponent(p.slug)}`}
                className="line-clamp-1 text-sm transition-colors hover:text-primary hover:underline"
              >
                {p.title}
              </Link>
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <span>{formatDate(p.published_at)}</span>
                {showViewCount && (
                  <span className="inline-flex items-center gap-0.5">
                    <EyeIcon className="size-3" />
                    {p.view_count}
                  </span>
                )}
                {showCommentCount && (
                  <span className="inline-flex items-center gap-0.5">
                    <MessageSquareIcon className="size-3" />
                    {p.comment_count}
                  </span>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </WidgetShell>
  )
}

/** 最新文章（order=recent，按发布时间倒序） */
export function RecentPostsWidget(props: WidgetProps) {
  return (
    <PostListWidget
      {...props}
      order="recent"
      defaultTitle="最新文章"
      icon={<FileTextIcon />}
    />
  )
}

/** 热门文章（order=hot，按浏览量→评论数倒序；副标题展示浏览数与评论数） */
export function HotPostsWidget(props: WidgetProps) {
  return (
    <PostListWidget
      {...props}
      order="hot"
      defaultTitle="热门文章"
      icon={<FlameIcon />}
      showViewCount
      showCommentCount
    />
  )
}
