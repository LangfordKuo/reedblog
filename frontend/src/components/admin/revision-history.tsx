import { useCallback, useEffect, useMemo, useState } from "react"
import { Loader2Icon, RotateCcwIcon } from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { api, errorMessage } from "@/lib/api"
import { diffLines } from "@/lib/diff"
import type { PostAdmin, PostRevision, PostRevisionSummary } from "@/lib/types"
import { cn, formatDateTime } from "@/lib/utils"

/**
 * 修订历史对话框（契约「文章修订历史」条款）：
 * 左侧列表（时间 + 标题 + 正文字符数），右侧选中修订与「当前编辑中正文」的行级差异
 * （未保存的修改也参与对比）；「恢复此版本」二次确认后写回并刷新列表（回滚留痕）。
 */
export function RevisionHistoryDialog({
  postId,
  currentContent,
  open,
  onOpenChange,
  onRestored,
}: {
  postId: number
  /** 当前编辑器中的正文（含未保存修改，直接作为差异右侧） */
  currentContent: string
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 恢复成功后回填编辑器（标题/正文/摘要） */
  onRestored: (post: PostAdmin) => void
}) {
  const [items, setItems] = useState<PostRevisionSummary[]>([])
  const [listLoading, setListLoading] = useState(false)
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [detail, setDetail] = useState<PostRevision | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [confirmOpen, setConfirmOpen] = useState(false)
  const [restoring, setRestoring] = useState(false)

  const loadList = useCallback(async (): Promise<PostRevisionSummary[]> => {
    setListLoading(true)
    try {
      const list = await api.admin.postRevisions(postId)
      setItems(list)
      // 尽量保持当前选中；选中项已被裁剪时回退到最新一条
      setSelectedId((prev) =>
        prev !== null && list.some((r) => r.id === prev) ? prev : (list[0]?.id ?? null),
      )
      return list
    } catch (e) {
      toast.error(errorMessage(e))
      return []
    } finally {
      setListLoading(false)
    }
  }, [postId])

  // 打开时拉列表；关闭时清空选中/详情，下次打开重新取最新状态
  useEffect(() => {
    if (!open) {
      setSelectedId(null)
      setDetail(null)
      setConfirmOpen(false)
      return
    }
    void loadList()
  }, [open, loadList])

  // 选中修订 → 拉取完整正文（列表摘要不含正文）
  useEffect(() => {
    if (!open || selectedId === null) {
      setDetail(null)
      return
    }
    let cancelled = false
    setDetailLoading(true)
    api.admin
      .postRevision(postId, selectedId)
      .then((d) => {
        if (!cancelled) setDetail(d)
      })
      .catch((e) => {
        if (!cancelled) toast.error(errorMessage(e))
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [open, postId, selectedId])

  const diff = useMemo(
    () => (detail ? diffLines(detail.content_md, currentContent) : []),
    [detail, currentContent],
  )
  const added = diff.filter((l) => l.type === "add").length
  const removed = diff.filter((l) => l.type === "del").length

  const handleRestore = async () => {
    if (selectedId === null || restoring) return
    setRestoring(true)
    try {
      const post = await api.admin.restorePostRevision(postId, selectedId)
      onRestored(post)
      toast.success("已恢复该版本，并生成一条新修订")
      setConfirmOpen(false)
      // 重新拉取并选中最新一条（即刚生成的修订，证明回滚留痕）
      const list = await loadList()
      setSelectedId(list[0]?.id ?? null)
    } catch (e) {
      toast.error(errorMessage(e))
    } finally {
      setRestoring(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !restoring && onOpenChange(o)}>
      <DialogContent className="sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle>修订历史</DialogTitle>
          <DialogDescription>
            每次内容变化的保存都会留档（每篇最多 20 条）；恢复会覆盖当前内容，同时生成一条新修订。
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 sm:grid-cols-[15rem_1fr]">
          {/* 修订列表（时间 + 标题 + 正文字符数） */}
          <div className="max-h-64 overflow-y-auto rounded-md border sm:max-h-[24rem]">
            {listLoading && (
              <div className="flex items-center gap-2 p-3 text-sm text-muted-foreground">
                <Loader2Icon className="size-4 animate-spin" />
                加载修订…
              </div>
            )}
            {!listLoading && items.length === 0 && (
              <p className="p-3 text-sm text-muted-foreground">暂无修订</p>
            )}
            {!listLoading &&
              items.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => setSelectedId(item.id)}
                  className={cn(
                    "flex w-full flex-col items-start gap-0.5 border-b px-3 py-2 text-left text-sm transition-colors last:border-b-0",
                    item.id === selectedId
                      ? "bg-accent text-accent-foreground"
                      : "hover:bg-accent/50",
                  )}
                >
                  <span className="line-clamp-1 font-medium">{item.title}</span>
                  <span className="text-xs text-muted-foreground">
                    {formatDateTime(item.created_at)} · {item.content_chars} 字符
                  </span>
                </button>
              ))}
          </div>

          {/* 差异对比（选中修订 vs 当前编辑中的正文） */}
          <div className="flex min-w-0 flex-col gap-3">
            {detailLoading ? (
              <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
                <Loader2Icon className="size-4 animate-spin" />
                加载正文…
              </div>
            ) : detail ? (
              <>
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="min-w-0">
                    <p className="line-clamp-1 text-sm font-medium">{detail.title}</p>
                    <p className="text-xs text-muted-foreground">
                      与当前编辑中的正文对比：
                      <span className="text-primary"> +{added} 行</span>
                      <span className="text-destructive"> -{removed} 行</span>
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={restoring}
                    onClick={() => setConfirmOpen(true)}
                  >
                    <RotateCcwIcon />
                    恢复此版本
                  </Button>
                </div>
                <div className="max-h-56 overflow-auto rounded-md border bg-muted/30 sm:max-h-[20rem]">
                  {added === 0 && removed === 0 ? (
                    <p className="p-3 text-sm text-muted-foreground">与当前正文无差异</p>
                  ) : (
                    diff.map((line, idx) => (
                      <div
                        key={idx}
                        className={cn(
                          "flex gap-2 px-2 py-px font-mono text-xs break-all whitespace-pre-wrap",
                          line.type === "add" && "bg-primary/10 text-primary",
                          line.type === "del" && "bg-destructive/10 text-destructive",
                        )}
                      >
                        <span className="w-3 shrink-0 select-none opacity-70">
                          {line.type === "add" ? "+" : line.type === "del" ? "-" : " "}
                        </span>
                        <span className="min-w-0">{line.text || " "}</span>
                      </div>
                    ))
                  )}
                </div>
              </>
            ) : (
              <p className="py-8 text-sm text-muted-foreground">
                选择左侧修订，查看与当前正文的差异
              </p>
            )}
          </div>
        </div>

        {/* 二次确认：明确「覆盖当前内容 + 生成一条新修订」 */}
        <ConfirmDialog
          open={confirmOpen}
          onOpenChange={setConfirmOpen}
          title="恢复此版本？"
          description="会覆盖当前编辑中的标题、正文与摘要（未保存的修改将丢失），同时生成一条新修订。"
          confirmText="恢复"
          loading={restoring}
          onConfirm={() => void handleRestore()}
        />
      </DialogContent>
    </Dialog>
  )
}
