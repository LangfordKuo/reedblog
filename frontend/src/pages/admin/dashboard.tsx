import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import {
  CheckCircle2Icon,
  FileTextIcon,
  FolderIcon,
  MessageSquareIcon,
  PenLineIcon,
  TagIcon,
} from "lucide-react"

import { PostStatusBadge } from "@/components/post-status-badge"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { api, errorMessage } from "@/lib/api"
import { formatDate, formatRelative } from "@/lib/utils"
import type { CommentAdmin, PostAdmin } from "@/lib/types"

interface Stats {
  posts: number
  published: number
  drafts: number
  comments: number
  categories: number
  tags: number
}

export default function DashboardPage() {
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [stats, setStats] = useState<Stats | null>(null)
  const [latestPosts, setLatestPosts] = useState<PostAdmin[]>([])
  const [latestComments, setLatestComments] = useState<CommentAdmin[]>([])

  useEffect(() => {
    document.title = "仪表盘 · reedblog"
    Promise.all([
      api.admin.posts({ status: "all", page: 1, per_page: 5 }),
      api.admin.posts({ status: "published", page: 1, per_page: 1 }),
      api.admin.posts({ status: "draft", page: 1, per_page: 1 }),
      api.admin.comments({ status: "all", page: 1, per_page: 5 }),
      api.admin.categories(),
      api.admin.tags(),
    ])
      .then(([all, published, drafts, comments, categories, tags]) => {
        setStats({
          posts: all.total,
          published: published.total,
          drafts: drafts.total,
          comments: comments.total,
          categories: categories.length,
          tags: tags.length,
        })
        setLatestPosts(all.items)
        setLatestComments(comments.items)
      })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [])

  if (loading) return <BlockSpinner label="加载仪表盘…" />
  if (error) {
    return <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">{error}</div>
  }

  return (
    <div className="flex flex-col gap-6">
      <h1 className="text-xl font-bold">仪表盘</h1>

      <div className="grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-6">
        <StatCard label="文章总数" value={stats?.posts ?? 0} icon={FileTextIcon} />
        <StatCard label="已发布" value={stats?.published ?? 0} icon={CheckCircle2Icon} />
        <StatCard label="草稿" value={stats?.drafts ?? 0} icon={PenLineIcon} />
        <StatCard label="评论" value={stats?.comments ?? 0} icon={MessageSquareIcon} />
        <StatCard label="分类" value={stats?.categories ?? 0} icon={FolderIcon} />
        <StatCard label="标签" value={stats?.tags ?? 0} icon={TagIcon} />
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">最近更新的文章</CardTitle>
          </CardHeader>
          <CardContent>
            {latestPosts.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                还没有文章，
                <Link to="/admin/posts/new" className="font-medium underline-offset-4 hover:underline">
                  写一篇吧
                </Link>
              </p>
            ) : (
              <ul className="flex flex-col gap-1">
                {latestPosts.map((p) => (
                  <li key={p.id}>
                    <Link
                      to={`/admin/posts/${p.id}/edit`}
                      className="flex items-center justify-between gap-3 rounded-md px-2 py-2 text-sm transition-colors hover:bg-accent"
                    >
                      <span className="truncate font-medium">{p.title}</span>
                      <span className="flex shrink-0 items-center gap-2">
                        <PostStatusBadge status={p.status} />
                        <span className="text-xs text-muted-foreground">
                          {formatDate(p.updated_at)}
                        </span>
                      </span>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">最近的评论</CardTitle>
          </CardHeader>
          <CardContent>
            {latestComments.length === 0 ? (
              <p className="text-sm text-muted-foreground">暂无评论</p>
            ) : (
              <ul className="flex flex-col gap-3">
                {latestComments.map((c) => (
                  <li key={c.id} className="flex flex-col gap-1 text-sm">
                    <div className="flex items-center gap-2">
                      <span className="font-medium">{c.author_name}</span>
                      <span className="text-xs text-muted-foreground">
                        {formatRelative(c.created_at)}
                      </span>
                      {c.status === "hidden" && (
                        <Badge variant="outline" className="text-xs font-normal">
                          已隐藏
                        </Badge>
                      )}
                    </div>
                    <p className="line-clamp-2 text-muted-foreground">{c.content}</p>
                    <p className="text-xs text-muted-foreground">
                      于《
                      <Link
                        to={`/admin/posts/${c.post_id}/edit`}
                        className="underline-offset-2 hover:underline"
                      >
                        {c.post_title}
                      </Link>
                      》
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  )
}

function StatCard({
  label,
  value,
  icon: Icon,
}: {
  label: string
  value: number
  icon: typeof FileTextIcon
}) {
  return (
    <Card className="gap-0 py-4">
      <CardContent className="flex flex-col gap-1 px-4">
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Icon className="size-3.5" />
          {label}
        </div>
        <div className="text-2xl font-bold">{value}</div>
      </CardContent>
    </Card>
  )
}
