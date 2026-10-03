import { useCallback, useEffect, useRef, useState } from "react"
import {
  DatabaseBackupIcon,
  DownloadIcon,
  Loader2Icon,
  TriangleAlertIcon,
  UploadIcon,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { api, errorMessage } from "@/lib/api"
import type { BackupInfo } from "@/lib/types"
import { formatBytes, formatDateTime } from "@/lib/utils"

/** 导入必须输入的确认字面量（契约「备份与恢复」：confirm 字段 = REPLACE） */
const CONFIRM_WORD = "REPLACE"

/**
 * 备份页（契约「备份与恢复」条款，2026-10-04 新增）：
 * - 导出：带 Bearer 的 fetch → Blob → 前端触发下载（不能用 <a href>，需带 Authorization 头）
 * - 导入：选 zip → 二次确认弹窗（输入 REPLACE）→ multipart 上传；失败展示后端返回的原因
 */
export default function AdminBackupPage() {
  const [info, setInfo] = useState<BackupInfo | null>(null)
  const [exporting, setExporting] = useState(false)
  const [file, setFile] = useState<File | null>(null)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [confirmText, setConfirmText] = useState("")
  const [importing, setImporting] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const loadInfo = useCallback(() => {
    api.admin
      .backupInfo()
      .then(setInfo)
      .catch(() => setInfo(null)) // 信息仅为展示，失败静默
  }, [])

  useEffect(loadInfo, [loadInfo])

  // 导出：Blob 下载（后端 Content-Disposition 带文件名，兜底前端生成）
  const handleExport = async () => {
    if (exporting) return
    setExporting(true)
    try {
      const { blob, filename } = await api.admin.exportBackup()
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = filename ?? `reedblog-backup-${Date.now()}.zip`
      document.body.appendChild(a)
      a.click()
      a.remove()
      URL.revokeObjectURL(url)
      toast.success(`备份已导出（${formatBytes(blob.size)}）`)
      loadInfo()
    } catch (e) {
      toast.error(errorMessage(e))
    } finally {
      setExporting(false)
    }
  }

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFile(e.target.files?.[0] ?? null)
  }

  const handleImport = async () => {
    if (!file || confirmText !== CONFIRM_WORD || importing) return
    setImporting(true)
    try {
      const result = await api.admin.importBackup(file, confirmText)
      const rows = Object.values(result.tables).reduce((a, b) => a + b, 0)
      toast.success(`导入完成：共恢复 ${rows} 行数据、${result.media_files} 个媒体文件`)
      setDialogOpen(false)
      setConfirmText("")
      setFile(null)
      if (fileInputRef.current) fileInputRef.current.value = ""
      loadInfo()
    } catch (e) {
      // 失败必须显示后端返回的原因（如 invalid_backup / confirmation_required 的 message）
      toast.error(errorMessage(e))
    } finally {
      setImporting(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-bold">备份</h1>
        <p className="text-sm text-muted-foreground">
          导出整站数据（数据库业务表 + 媒体文件）为 zip，或从备份文件恢复
        </p>
      </div>

      <div className="grid gap-6 md:grid-cols-2">
        {/* 导出 */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <DownloadIcon className="size-4" />
              导出备份
            </CardTitle>
            <CardDescription>
              包含文章（含回收站）、修订、评论、页面、分类标签、设置、用户（含密码哈希）
              与全部媒体文件；不含 config.toml（jwt secret、数据库与 SMTP 密码）。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <div className="rounded-md border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
              {info?.last_export_at
                ? `最近导出：${formatDateTime(info.last_export_at)}${
                    info.total_size_bytes !== null
                      ? `（${formatBytes(info.total_size_bytes)}）`
                      : ""
                  }`
                : "本次运行尚未导出过备份"}
            </div>
            <Button className="self-start" disabled={exporting} onClick={() => void handleExport()}>
              {exporting ? <Loader2Icon className="animate-spin" /> : <DownloadIcon />}
              {exporting ? "正在打包…" : "导出并下载 zip"}
            </Button>
            <p className="text-xs text-muted-foreground">
              备份文件含管理员密码哈希，请妥善保管。
            </p>
          </CardContent>
        </Card>

        {/* 导入 */}
        <Card className="border-destructive/40">
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-destructive">
              <TriangleAlertIcon className="size-4" />
              导入恢复
            </CardTitle>
            <CardDescription>
              会用备份文件覆盖当前全部内容（文章、评论、页面、媒体、设置与用户），
              <span className="font-medium text-destructive">且不可撤销</span>。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <Input
              ref={fileInputRef}
              type="file"
              accept=".zip,application/zip"
              disabled={importing}
              onChange={handleFileChange}
              aria-label="选择备份 zip 文件"
            />
            <Button
              variant="destructive"
              className="self-start"
              disabled={!file || importing}
              onClick={() => {
                setConfirmText("")
                setDialogOpen(true)
              }}
            >
              <UploadIcon />
              导入并覆盖
            </Button>
            <p className="text-xs text-muted-foreground">
              导入不会恢复 config.toml（jwt secret 等保持现状，已登录会话继续有效）。
            </p>
          </CardContent>
        </Card>
      </div>

      {/* 二次确认弹窗：必须输入 REPLACE 才能执行 */}
      <Dialog open={dialogOpen} onOpenChange={(o) => !importing && setDialogOpen(o)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-destructive">
              <DatabaseBackupIcon className="size-4" />
              确认导入备份
            </DialogTitle>
            <DialogDescription>
              这会用备份文件覆盖当前全部内容，且不可撤销。请输入
              <span className="mx-1 font-mono font-semibold text-foreground">{CONFIRM_WORD}</span>
              以确认。
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2">
            <Input
              value={confirmText}
              disabled={importing}
              placeholder={CONFIRM_WORD}
              autoComplete="off"
              onChange={(e) => setConfirmText(e.target.value)}
              aria-label={`输入 ${CONFIRM_WORD} 确认导入`}
            />
            {file && (
              <p className="truncate text-xs text-muted-foreground">
                待导入文件：{file.name}（{formatBytes(file.size)}）
              </p>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" disabled={importing} onClick={() => setDialogOpen(false)}>
              取消
            </Button>
            <Button
              variant="destructive"
              disabled={confirmText !== CONFIRM_WORD || importing}
              onClick={() => void handleImport()}
            >
              {importing && <Loader2Icon className="animate-spin" />}
              {importing ? "正在导入…" : "确认覆盖导入"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
