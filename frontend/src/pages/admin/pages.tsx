import { useCallback, useEffect, useState } from "react"
import { useNavigate } from "react-router-dom"
import { ExternalLinkIcon, Loader2Icon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Switch } from "@/components/ui/switch"
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
import type { PageAdmin, PageKind } from "@/lib/types"

/** kind 徽标：类型不可改，仅展示 */
function PageKindBadge({ kind }: { kind: PageKind }) {
  switch (kind) {
    case "message_board":
      return <Badge className="border-sky-200 bg-sky-50 text-sky-700">留言板</Badge>
    case "links":
      return <Badge className="border-violet-200 bg-violet-50 text-violet-700">友情链接</Badge>
    default:
      return <Badge variant="outline">普通页</Badge>
  }
}

export default function AdminPagesPage() {
  const navigate = useNavigate()
  const [pages, setPages] = useState<PageAdmin[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [togglingId, setTogglingId] = useState<number | null>(null)
  const [deleting, setDeleting] = useState<PageAdmin | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)

  const load = useCallback(() => {
    api.admin
      .pages()
      .then((d) => {
        setPages(d)
        setError(null)
      })
      .catch((e) => setError(errorMessage(e)))
  }, [])

  useEffect(load, [load])

  /** 启用开关直接切换（PATCH toggle；停用后前台立即 404、导航消失） */
  const toggleEnabled = async (p: PageAdmin) => {
    setTogglingId(p.id)
    try {
      const updated = await api.admin.togglePage(p.id)
      setPages((prev) => prev?.map((it) => (it.id === updated.id ? updated : it)) ?? prev)
      toast.success(updated.enabled ? `「${updated.title}」已启用` : `「${updated.title}」已停用`)
    } catch (err) {
      toast.error(errorMessage(err))
    } finally {
      setTogglingId(null)
    }
  }

  const handleDelete = async () => {
    if (!deleting) return
    setDeleteLoading(true)
    try {
      await api.admin.deletePage(deleting.id)
      toast.success(`已删除「${deleting.title}」`)
      setDeleting(null)
      load()
    } catch (err) {
      // 契约：built_in → 422 page_builtin（正常情况下按钮已禁用，这里是兜底）
      toast.error(errorMessage(err, { page_builtin: "内置页面不可删除，可将其停用" }))
      setDeleting(null)
    } finally {
      setDeleteLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">页面管理</h1>
          <p className="text-sm text-muted-foreground">
            共 {pages?.length ?? 0} 个页面；启用的页面会出现在前台顶栏导航与 sitemap
          </p>
        </div>
        <Button onClick={() => navigate("/admin/pages/new")}>
          <PlusIcon />
          新建页面
        </Button>
      </div>

      {pages === null ? (
        error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
            {error}
          </div>
        ) : (
          <BlockSpinner />
        )
      ) : pages.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-12 text-sm text-muted-foreground">
            <p>还没有页面</p>
            <Button variant="outline" onClick={() => navigate("/admin/pages/new")}>
              <PlusIcon />
              新建页面
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
                  <TableHead>类型</TableHead>
                  <TableHead>排序</TableHead>
                  <TableHead>更新时间</TableHead>
                  <TableHead>启用</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {pages.map((p) => (
                  <TableRow key={p.id}>
                    <TableCell className="max-w-64">
                      <div className="flex items-center gap-2">
                        <button
                          type="button"
                          onClick={() => navigate(`/admin/pages/${p.id}/edit`)}
                          className="truncate font-medium underline-offset-4 hover:underline"
                        >
                          {p.title}
                        </button>
                        {p.built_in && (
                          <Badge variant="secondary" className="shrink-0 text-xs">
                            内置
                          </Badge>
                        )}
                      </div>
                      <span className="block truncate text-xs text-muted-foreground">
                        /pages/{p.slug}
                      </span>
                    </TableCell>
                    <TableCell>
                      <PageKindBadge kind={p.kind} />
                    </TableCell>
                    <TableCell className="text-muted-foreground">{p.sort_order}</TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(p.updated_at)}
                    </TableCell>
                    <TableCell>
                      {togglingId === p.id ? (
                        <Loader2Icon className="size-4 animate-spin text-muted-foreground" />
                      ) : (
                        <Switch
                          checked={p.enabled}
                          aria-label={`${p.enabled ? "停用" : "启用"}页面 ${p.title}`}
                          onCheckedChange={() => void toggleEnabled(p)}
                        />
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        {p.enabled && (
                          <Button variant="ghost" size="icon" aria-label="查看页面" asChild>
                            <a
                              href={`/pages/${encodeURIComponent(p.slug)}`}
                              target="_blank"
                              rel="noreferrer"
                            >
                              <ExternalLinkIcon />
                            </a>
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="编辑"
                          onClick={() => navigate(`/admin/pages/${p.id}/edit`)}
                        >
                          <PencilIcon />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="删除"
                          className="text-destructive hover:text-destructive"
                          // 契约：built_in 页面不可删除（后端 422 page_builtin），前端直接禁用
                          disabled={p.built_in}
                          title={p.built_in ? "内置页面不可删除，可将其停用" : "删除"}
                          onClick={() => setDeleting(p)}
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

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(o) => !o && setDeleting(null)}
        title="删除页面"
        description={`确定删除「${deleting?.title ?? ""}」吗？该页的友情链接与留言将一并删除，此操作不可撤销。`}
        loading={deleteLoading}
        onConfirm={handleDelete}
      />
    </div>
  )
}
