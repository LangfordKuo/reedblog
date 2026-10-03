import { useEffect, useState } from "react"
import { Link } from "react-router-dom"

import { api } from "@/lib/api"
import { formatDate } from "@/lib/utils"
import type { PostPublic } from "@/lib/types"

/** 区块请求条数（契约「相关文章推荐」条款：默认 5，上限 10） */
const RELATED_LIMIT = 5

/** 文章详情底部「相关文章」列表（契约「相关文章推荐」条款）：
 *  最多 5 条，每条为标题链接 + 发布日期。加载完成前不渲染（避免布局跳动）；
 *  拉取失败或结果为空时整块不渲染（静默降级，不报错不占位）。
 *  单列列表：标题多为长中文句子，双列网格在窄屏会被压成窄条或溢出。 */
export function RelatedPosts({ slug }: { slug: string }) {
  const [posts, setPosts] = useState<PostPublic[] | null>(null)

  useEffect(() => {
    let cancelled = false
    setPosts(null)
    api
      .related(slug, RELATED_LIMIT)
      .then((items) => {
        if (!cancelled) setPosts(items)
      })
      .catch(() => {
        // 静默降级：失败与空结果同样不渲染（详情接口自身的错误在页面层另行处理）
        if (!cancelled) setPosts([])
      })
    return () => {
      cancelled = true
    }
  }, [slug])

  if (!posts || posts.length === 0) return null

  return (
    <section aria-label="相关文章" className="mt-10">
      <h2 className="text-base font-semibold tracking-tight">相关文章</h2>
      <ul className="mt-3 flex flex-col gap-2">
        {posts.map((p) => (
          <li key={p.id}>
            <Link
              to={`/posts/${encodeURIComponent(p.slug)}`}
              className="flex min-w-0 items-center justify-between gap-4 rounded-lg border bg-card px-4 py-3 transition-colors hover:border-foreground/25 hover:bg-accent/50"
            >
              <span className="min-w-0 line-clamp-2 text-sm font-medium break-words">
                {p.title}
              </span>
              <time className="shrink-0 text-xs text-muted-foreground" dateTime={p.published_at}>
                {formatDate(p.published_at)}
              </time>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  )
}
