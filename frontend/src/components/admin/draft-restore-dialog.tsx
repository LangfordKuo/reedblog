import { RotateCcwIcon, Trash2Icon } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { formatDraftAge, formatDraftClock } from "@/lib/draft"

/**
 * 本地草稿恢复提示：打开编辑器时若发现与服务端版本不同的本地草稿，
 * 由用户明确选择「恢复 / 丢弃」，绝不做静默覆盖。
 * 关闭（X / Esc / 点遮罩）视为「稍后决定」：草稿保留，下次打开仍会提示。
 */
export function DraftRestoreDialog({
  open,
  savedAt,
  onOpenChange,
  onRestore,
  onDiscard,
}: {
  open: boolean
  /** 草稿写入时刻（epoch ms） */
  savedAt: number
  onOpenChange: (open: boolean) => void
  onRestore: () => void
  onDiscard: () => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>检测到未保存的本地草稿</DialogTitle>
          <DialogDescription>
            保存于 {formatDraftAge(savedAt)}
            {formatDraftClock(savedAt) ? `（${formatDraftClock(savedAt)}）` : ""}
            ，与服务器上的版本不同。是否恢复到编辑器？
            <br />
            恢复只影响当前编辑内容，不会自动覆盖服务器；恢复后仍需点击「保存」才会上传。
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onDiscard}>
            <Trash2Icon />
            丢弃草稿
          </Button>
          <Button onClick={onRestore}>
            <RotateCcwIcon />
            恢复草稿
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
