import { useCallback, useEffect, useState } from "react"
import { Link, useSearchParams } from "react-router-dom"
import {
  CheckIcon,
  CornerDownRightIcon,
  EyeIcon,
  EyeOffIcon,
  Loader2Icon,
  Trash2Icon,
} from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Pagination } from "@/components/pagination"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
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
import { formatDateTime } from "@/lib/utils"
import type { CommentAdmin, CommentStatus, Page } from "@/lib/types"

type StatusFilter = CommentStatus | "all"

const PER_PAGE = 10

export default function AdminCommentsPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const status = (searchParams.get("status") ?? "all") as StatusFilter
  const page = Math.max(1, Number(searchParams.get("page")) || 1)

  const [data, setData] = useState<Page<CommentAdmin> | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [togglingId, setTogglingId] = useState<number | null>(null)
  const [deleting, setDeleting] = useState<CommentAdmin | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)
  // 待审计数（契约「评论审核方式」：不加 SiteStats 字段，用管理接口 status=pending 查询）
  const [pendingCount, setPendingCount] = useState(0)
  const [approveAllOpen, setApproveAllOpen] = useState(false)
  const [approveAllLoading, setApproveAllLoading] = useState(false)

  /** 刷新待审条数（顶部提示与「全部通过」按钮显示条件） */
  const refreshPendingCount = useCallback(() => {
    api.admin
      .comments({ status: "pending", page: 1, per_page: 1 })
      .then((d) => setPendingCount(d.total))
      .catch(() => {})
  }, [])

  const load = useCallback(() => {
    setLoading(true)
    refreshPendingCount()
    api.admin
      .comments({ status, page, per_page: PER_PAGE })
      .then((d) => {
        setData(d)
        setError(null)
      })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [status, page, refreshPendingCount])

  useEffect(load, [load])

  const updateParams = (patch: Record<string, string | null>) => {
    const next = new URLSearchParams(searchParams)
    for (const [k, v] of Object.entries(patch)) {
      if (v === null) next.delete(k)
      else next.set(k, v)
    }
    setSearchParams(next, { replace: true })
  }

  const toggleStatus = async (c: CommentAdmin) => {
    const next: CommentStatus = c.status === "approved" ? "hidden" : "approved"
    setTogglingId(c.id)
    try {
      const updated = await api.admin.updateComment(c.id, next)
      setData((prev) =>
        prev ? { ...prev, items: prev.items.map((it) => (it.id === updated.id ? updated : it)) } : prev,
      )
      toast.success(next === "hidden" ? "评论已隐藏" : "评论已恢复")
    } catch (err) {
      toast.error(errorMessage(err))
    } finally {
      setTogglingId(null)
    }
  }

  /**
   * 后台「通过」待审评论（契约「评论审核方式」）：PUT {status:"approved"}，
   * 复用既有改状态端点；通过时不发额外邮件，公开列表/计数实时生效。
   */
  const approve = async (c: CommentAdmin) => {
    setTogglingId(c.id)
    try {
      const updated = await api.admin.updateComment(c.id, "approved")
      if (status === "pending") {
        load() // 待审筛选下通过后该行应移出列表
      } else {
        setData((prev) =>
          prev
            ? { ...prev, items: prev.items.map((it) => (it.id === updated.id ? updated : it)) }
            : prev,
        )
      }
      refreshPendingCount()
      toast.success("评论已通过")
    } catch (err) {
      toast.error(errorMessage(err))
    } finally {
      setTogglingId(null)
    }
  }

  /** 「全部通过」（加分项）：二次确认后分页拉取全部待审并逐条通过，循环直至清空 */
  const approveAll = async () => {
    setApproveAllLoading(true)
    let approved = 0
    try {
      // 每轮取第一页最多 100 条：通过后即脱离 pending 筛选，循环直至没有剩余
      for (;;) {
        const d = await api.admin.comments({ status: "pending", page: 1, per_page: 100 })
        if (d.items.length === 0) break
        for (const it of d.items) {
          await api.admin.updateComment(it.id, "approved")
          approved += 1
        }
      }
      toast.success(approved > 0 ? `已通过 ${approved} 条待审评论` : "没有待审评论")
      setApproveAllOpen(false)
      load()
    } catch (err) {
      toast.error(`操作中断：${errorMessage(err)}`)
      load()
    } finally {
      setApproveAllLoading(false)
    }
  }

  const handleDelete = async () => {
    if (!deleting) return
    setDeleteLoading(true)
    try {
      await api.admin.deleteComment(deleting.id)
      toast.success("评论已删除")
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
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">评论管理</h1>
          <p className="text-sm text-muted-foreground">共 {data?.total ?? 0} 条评论</p>
        </div>
        {pendingCount > 0 && (
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => updateParams({ status: "pending", page: null })}
            >
              待审核 {pendingCount} 条
            </Button>
            <Button size="sm" onClick={() => setApproveAllOpen(true)}>
              <CheckIcon className="size-4" />
              全部通过
            </Button>
          </div>
        )}
      </div>

      <Tabs
        value={status}
        onValueChange={(v) => updateParams({ status: v === "all" ? null : v, page: null })}
      >
        <TabsList>
          <TabsTrigger value="all">全部</TabsTrigger>
          <TabsTrigger value="pending">待审核</TabsTrigger>
          <TabsTrigger value="approved">已展示</TabsTrigger>
          <TabsTrigger value="hidden">已隐藏</TabsTrigger>
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
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            没有符合条件的评论
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>来源</TableHead>
                  <TableHead>作者</TableHead>
                  <TableHead>内容</TableHead>
                  <TableHead>时间</TableHead>
                  <TableHead>状态</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data?.items.map((c) => (
                  <TableRow key={c.id}>
                    <TableCell className="max-w-40">
                      {/* 来源（契约「页面」条款）：target_type 区分文章评论 / 页面留言 */}
                      <div className="flex items-center gap-1.5">
                        <Badge
                          variant="outline"
                          className={
                            c.target_type === "page"
                              ? "shrink-0 border-sky-200 bg-sky-50 text-sky-700"
                              : "shrink-0 text-muted-foreground"
                          }
                        >
                          {c.target_type === "page" ? "页面" : "文章"}
                        </Badge>
                        <Link
                          to={
                            c.target_type === "page"
                              ? `/admin/pages/${c.post_id}/edit`
                              : `/admin/posts/${c.post_id}/edit`
                          }
                          className="truncate underline-offset-4 hover:underline"
                        >
                          {c.post_title}
                        </Link>
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="font-medium">{c.author_name}</div>
                      {c.email && (
                        <div className="max-w-40 truncate text-xs text-muted-foreground">
                          {c.email}
                        </div>
                      )}
                    </TableCell>
                    <TableCell className="max-w-64 whitespace-normal">
                      {/* 回复关系（契约「评论回复」条款）：子回复缩进 + 「回复 @xxx」标记 */}
                      <div className={c.parent_id !== null ? "border-l-2 pl-2" : undefined}>
                        {c.parent_id !== null && (
                          <div className="mb-0.5 flex items-center gap-1 text-xs text-muted-foreground">
                            <CornerDownRightIcon className="size-3 shrink-0" />
                            <span className="truncate">
                              回复 {c.reply_to_name ? `@${c.reply_to_name}` : "楼层"}
                            </span>
                          </div>
                        )}
                        <p className="line-clamp-2 text-muted-foreground">{c.content}</p>
                        {c.reply_count > 0 && (
                          <div className="mt-0.5 text-xs text-muted-foreground">
                            {c.reply_count} 条回复
                          </div>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDateTime(c.created_at)}
                    </TableCell>
                    <TableCell>
                      {c.status === "pending" ? (
                        // 待审醒目标记（契约「评论审核方式」）
                        <Badge className="border-amber-300 bg-amber-50 text-amber-700">
                          待审核
                        </Badge>
                      ) : c.status === "approved" ? (
                        <Badge className="border-emerald-200 bg-emerald-50 text-emerald-700">
                          已展示
                        </Badge>
                      ) : (
                        <Badge variant="outline" className="text-muted-foreground">
                          已隐藏
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        {c.status === "pending" ? (
                          <Button
                            variant="ghost"
                            size="icon"
                            disabled={togglingId === c.id}
                            aria-label="通过"
                            title="通过审核"
                            onClick={() => void approve(c)}
                          >
                            {togglingId === c.id ? (
                              <Loader2Icon className="animate-spin" />
                            ) : (
                              <CheckIcon />
                            )}
                          </Button>
                        ) : (
                          <Button
                            variant="ghost"
                            size="icon"
                            disabled={togglingId === c.id}
                            aria-label={c.status === "approved" ? "隐藏" : "恢复"}
                            title={c.status === "approved" ? "隐藏" : "恢复展示"}
                            onClick={() => void toggleStatus(c)}
                          >
                            {togglingId === c.id ? (
                              <Loader2Icon className="animate-spin" />
                            ) : c.status === "approved" ? (
                              <EyeOffIcon />
                            ) : (
                              <EyeIcon />
                            )}
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="删除"
                          className="text-destructive hover:text-destructive"
                          onClick={() => setDeleting(c)}
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

      {/* 「全部通过」二次确认（契约「评论审核方式」；操作会公开全部待审评论） */}
      <ConfirmDialog
        open={approveAllOpen}
        onOpenChange={(o) => !o && setApproveAllOpen(false)}
        title="全部通过"
        description={`确定通过全部 ${pendingCount} 条待审评论吗？通过后它们会立即在公开页面显示。`}
        loading={approveAllLoading}
        onConfirm={approveAll}
      />

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(o) => !o && setDeleting(null)}
        title="删除评论"
        description={
          // 连带删除提示（契约「评论回复」条款）：删顶级评论会连带删除其全部子回复
          deleting && deleting.reply_count > 0
            ? `确定删除 ${deleting.author_name} 的这条评论吗？将连带删除其 ${deleting.reply_count} 条回复。此操作不可撤销。`
            : `确定删除 ${deleting?.author_name ?? ""} 的这条评论吗？此操作不可撤销。`
        }
        loading={deleteLoading}
        onConfirm={handleDelete}
      />
    </div>
  )
}
