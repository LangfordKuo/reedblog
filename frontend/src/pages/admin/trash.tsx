import { useCallback, useEffect, useState } from "react"
import { useNavigate, useSearchParams } from "react-router-dom"
import { ArrowLeftIcon, Loader2Icon, RotateCcwIcon, Trash2Icon } from "lucide-react"
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
import { api, errorMessage } from "@/lib/api"
import { formatDate } from "@/lib/utils"
import type { Page, PostAdmin } from "@/lib/types"

const PER_PAGE = 10

/**
 * 文章回收站（契约「文章回收站」条款，2026-10-04 新增）：
 * 列出软删除文章（deleted_at 非 null，按 deleted_at DESC, id DESC），支持恢复与彻底删除；
 * 彻底删除需二次确认（明确提示评论/点赞一并删除、不可恢复）。
 */
export default function AdminTrashPage() {
  const navigate = useNavigate()
  const [searchParams, setSearchParams] = useSearchParams()
  const page = Math.max(1, Number(searchParams.get("page")) || 1)

  const [data, setData] = useState<Page<PostAdmin> | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [restoringId, setRestoringId] = useState<number | null>(null)
  const [purging, setPurging] = useState<PostAdmin | null>(null)
  const [purgeLoading, setPurgeLoading] = useState(false)

  const load = useCallback(() => {
    setLoading(true)
    api.admin
      .trashPosts({ page, per_page: PER_PAGE })
      .then((d) => {
        setData(d)
        setError(null)
      })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [page])

  useEffect(load, [load])

  const goPage = (p: number) => {
    const next = new URLSearchParams(searchParams)
    if (p <= 1) next.delete("page")
    else next.set("page", String(p))
    setSearchParams(next, { replace: true })
  }

  // 恢复：非破坏性操作，无需二次确认；恢复后前台按原可见性规则重新可见
  const handleRestore = async (post: PostAdmin) => {
    if (restoringId !== null) return
    setRestoringId(post.id)
    try {
      await api.admin.restorePost(post.id)
      toast.success(`已恢复「${post.title}」`)
      load()
    } catch (err) {
      toast.error(errorMessage(err))
    } finally {
      setRestoringId(null)
    }
  }

  const handlePurge = async () => {
    if (!purging) return
    setPurgeLoading(true)
    try {
      await api.admin.purgePost(purging.id)
      toast.success(`已彻底删除「${purging.title}」`)
      setPurging(null)
      load()
    } catch (err) {
      toast.error(errorMessage(err))
      setPurging(null)
    } finally {
      setPurgeLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">回收站</h1>
          <p className="text-sm text-muted-foreground">
            共 {data?.total ?? 0} 篇文章 · 回收站中的文章不会出现在前台，可恢复或彻底删除
          </p>
        </div>
        <Button variant="outline" onClick={() => navigate("/admin/posts")}>
          <ArrowLeftIcon />
          返回文章管理
        </Button>
      </div>

      {loading ? (
        <BlockSpinner />
      ) : error ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
          {error}
        </div>
      ) : data && data.items.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-12 text-sm text-muted-foreground">
            <p>回收站是空的</p>
            <Button variant="outline" onClick={() => navigate("/admin/posts")}>
              <ArrowLeftIcon />
              返回文章管理
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
                  <TableHead>原状态</TableHead>
                  <TableHead>删除时间</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data?.items.map((post) => (
                  <TableRow key={post.id}>
                    <TableCell className="max-w-64">
                      <span className="block truncate font-medium">{post.title}</span>
                      <span className="block truncate text-xs text-muted-foreground">
                        /{post.slug}
                      </span>
                    </TableCell>
                    <TableCell>
                      <PostStatusBadge status={post.status} />
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {post.deleted_at ? formatDate(post.deleted_at) : "—"}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={restoringId !== null}
                          onClick={() => void handleRestore(post)}
                        >
                          {restoringId === post.id ? (
                            <Loader2Icon className="animate-spin" />
                          ) : (
                            <RotateCcwIcon />
                          )}
                          恢复
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-destructive hover:text-destructive"
                          onClick={() => setPurging(post)}
                        >
                          <Trash2Icon />
                          彻底删除
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
          onChange={goPage}
        />
      )}

      {/* 彻底删除二次确认（契约要求文案明确「不可恢复 + 评论点赞一并删除」） */}
      <ConfirmDialog
        open={purging !== null}
        onOpenChange={(o) => !o && setPurging(null)}
        title="彻底删除文章"
        description={
          // 目标为空（弹窗关闭/删除完成）时不渲染正文，避免把 undefined 拼进 DOM 文案
          purging
            ? `确定彻底删除「${purging.title}」吗？彻底删除不可恢复，该文章的评论与点赞也会一并删除。`
            : ""
        }
        confirmText="彻底删除"
        loading={purgeLoading}
        onConfirm={handlePurge}
      />
    </div>
  )
}
