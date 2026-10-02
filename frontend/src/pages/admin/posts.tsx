import { useCallback, useEffect, useState } from "react"
import { Link, useNavigate, useSearchParams } from "react-router-dom"
import { ExternalLinkIcon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Pagination } from "@/components/pagination"
import { PostStatusBadge } from "@/components/post-status-badge"
import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { api, errorMessage } from "@/lib/api"
import { formatDate } from "@/lib/utils"
import type { Page, PostAdmin, PostStatus } from "@/lib/types"

type StatusFilter = PostStatus | "all"

const PER_PAGE = 10

export default function AdminPostsPage() {
  const navigate = useNavigate()
  const [searchParams, setSearchParams] = useSearchParams()
  const status = (searchParams.get("status") ?? "all") as StatusFilter
  const page = Math.max(1, Number(searchParams.get("page")) || 1)

  const [data, setData] = useState<Page<PostAdmin> | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<PostAdmin | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)

  const load = useCallback(() => {
    setLoading(true)
    api.admin
      .posts({ status, page, per_page: PER_PAGE })
      .then((d) => {
        setData(d)
        setError(null)
      })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [status, page])

  useEffect(load, [load])

  const updateParams = (patch: Record<string, string | null>) => {
    const next = new URLSearchParams(searchParams)
    for (const [k, v] of Object.entries(patch)) {
      if (v === null) next.delete(k)
      else next.set(k, v)
    }
    setSearchParams(next, { replace: true })
  }

  const handleDelete = async () => {
    if (!deleting) return
    setDeleteLoading(true)
    try {
      await api.admin.deletePost(deleting.id)
      toast.success(`已删除「${deleting.title}」`)
      setDeleting(null)
      load()
    } catch (err) {
      toast.error(errorMessage(err))
      setDeleting(null)
    } finally {
      setDeleteLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">文章管理</h1>
          <p className="text-sm text-muted-foreground">共 {data?.total ?? 0} 篇文章</p>
        </div>
        <Button onClick={() => navigate("/admin/posts/new")}>
          <PlusIcon />
          新建文章
        </Button>
      </div>

      <Tabs
        value={status}
        onValueChange={(v) => updateParams({ status: v === "all" ? null : v, page: null })}
      >
        <TabsList>
          <TabsTrigger value="all">全部</TabsTrigger>
          <TabsTrigger value="published">已发布</TabsTrigger>
          <TabsTrigger value="draft">草稿</TabsTrigger>
        </TabsList>
      </Tabs>

      {loading ? (
        <BlockSpinner />
      ) : error ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
          {error}
        </div>
      ) : data && data.items.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-12 text-sm text-muted-foreground">
            <p>没有符合条件的文章</p>
            <Button variant="outline" onClick={() => navigate("/admin/posts/new")}>
              <PlusIcon />
              新建文章
            </Button>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>标题</TableHead>
                  <TableHead>状态</TableHead>
                  <TableHead>分类</TableHead>
                  <TableHead>更新时间</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data?.items.map((post) => (
                  <TableRow key={post.id}>
                    <TableCell className="max-w-64">
                      <Link
                        to={`/admin/posts/${post.id}/edit`}
                        className="block truncate font-medium underline-offset-4 hover:underline"
                      >
                        {post.title}
                      </Link>
                      <span className="block truncate text-xs text-muted-foreground">
                        /{post.slug}
                      </span>
                    </TableCell>
                    <TableCell>
                      <PostStatusBadge status={post.status} />
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {post.category_name ?? "—"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(post.updated_at)}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        {post.status === "published" && (
                          <Button variant="ghost" size="icon" aria-label="查看文章" asChild>
                            <a href={`/posts/${encodeURIComponent(post.slug)}`} target="_blank" rel="noreferrer">
                              <ExternalLinkIcon />
                            </a>
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="编辑"
                          onClick={() => navigate(`/admin/posts/${post.id}/edit`)}
                        >
                          <PencilIcon />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="删除"
                          className="text-destructive hover:text-destructive"
                          onClick={() => setDeleting(post)}
                        >
                          <Trash2Icon />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {data && (
        <Pagination
          page={data.page}
          perPage={data.per_page}
          total={data.total}
          onChange={(p) => updateParams({ page: p <= 1 ? null : String(p) })}
        />
      )}

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(o) => !o && setDeleting(null)}
        title="删除文章"
        description={`确定删除「${deleting?.title}」吗？此操作不可撤销。`}
        loading={deleteLoading}
        onConfirm={handleDelete}
      />
    </div>
  )
}
