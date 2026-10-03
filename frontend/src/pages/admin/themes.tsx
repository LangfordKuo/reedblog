import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import { useNavigate } from "react-router-dom"
import { CheckIcon, ImageIcon, Loader2Icon, Settings2Icon, Trash2Icon, UploadIcon } from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { api, errorMessage } from "@/lib/api"
import { applyActiveTheme } from "@/lib/theme"
import type { ThemeInfo } from "@/lib/types"

// 上传/删除错误码 → 用户文案（契约：docs/extensibility-contract.md 主题管理 API）
const UPLOAD_ERR_MAP: Record<string, string> = {
  theme_exists: "已存在同 slug 的主题，请先删除旧主题或修改包内 slug",
  builtin_protected: "内置 default 主题不可被上传覆盖",
  invalid_manifest: "theme.toml 不合法：字段格式或设计令牌有误",
  invalid_package: "zip 包不合法：根层须为单个主题目录且包含 theme.toml",
}

const DELETE_ERR_MAP: Record<string, string> = {
  builtin_protected: "内置主题不可删除",
  theme_active: "当前激活主题不可删除，请先切换到其他主题",
}

/** builtin / active 主题的删除按钮禁用原因 */
function deleteDisabledHint(t: ThemeInfo): string | null {
  if (t.builtin) return "内置主题不可删除"
  if (t.active) return "当前激活主题不可删除，请先切换主题"
  return null
}

export default function AdminThemesPage() {
  const navigate = useNavigate()
  const [items, setItems] = useState<ThemeInfo[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [file, setFile] = useState<File | null>(null)
  const [uploading, setUploading] = useState(false)
  const [activatingSlug, setActivatingSlug] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<ThemeInfo | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  const load = useCallback(() => {
    api.admin
      .themes()
      .then((d) => {
        setItems(d.items)
        setError(null)
      })
      .catch((e) => {
        setItems([])
        setError(errorMessage(e))
      })
  }, [])

  useEffect(load, [load])

  const handleUpload = async (e: FormEvent) => {
    e.preventDefault()
    if (!file || uploading) return
    setUploading(true)
    try {
      const created = await api.admin.uploadTheme(file)
      setItems((prev) => [...(prev ?? []), created])
      toast.success(`主题「${created.name}」已上传，激活后生效`)
      setFile(null)
      if (fileRef.current) fileRef.current.value = ""
    } catch (err) {
      toast.error(errorMessage(err, UPLOAD_ERR_MAP))
    } finally {
      setUploading(false)
    }
  }

  const handleActivate = async (t: ThemeInfo) => {
    setActivatingSlug(t.slug)
    try {
      const updated = await api.admin.activateTheme(t.slug)
      setItems((prev) =>
        (prev ?? []).map((it) => ({ ...it, active: it.slug === updated.slug })),
      )
      toast.success(`主题「${updated.name}」已激活，前台下次加载生效`)
      // 立即重拉 active 主题热切换当前页面样式（契约允许，非强制）
      void applyActiveTheme()
    } catch (err) {
      toast.error(errorMessage(err))
    } finally {
      setActivatingSlug(null)
    }
  }

  const handleDelete = async () => {
    if (!deleting) return
    setDeleteLoading(true)
    try {
      await api.admin.deleteTheme(deleting.slug)
      setItems((prev) => (prev ?? []).filter((it) => it.slug !== deleting.slug))
      toast.success(`主题「${deleting.name}」已删除`)
      setDeleting(null)
    } catch (err) {
      toast.error(errorMessage(err, DELETE_ERR_MAP))
      setDeleting(null)
      load() // 失败后重新拉取，保证列表与服务端一致
    } finally {
      setDeleteLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-bold">主题管理</h1>
        <p className="text-sm text-muted-foreground">
          共 {items?.length ?? 0} 套主题；上传 zip（根层为单个主题目录，含 theme.toml），
          激活后前台下次加载生效。内置 default 主题与当前激活主题不可删除。
        </p>
      </div>

      <Card>
        <CardContent>
          <form onSubmit={handleUpload} className="flex flex-wrap items-center gap-2">
            <Input
              ref={fileRef}
              type="file"
              accept=".zip,application/zip"
              className="max-w-sm"
              onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            />
            <Button type="submit" disabled={uploading || !file}>
              {uploading ? <Loader2Icon className="animate-spin" /> : <UploadIcon />}
              上传主题
            </Button>
          </form>
        </CardContent>
      </Card>

      {items === null ? (
        <BlockSpinner />
      ) : error ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
          {error}
        </div>
      ) : items.length === 0 ? (
        <div className="py-12 text-center text-sm text-muted-foreground">暂无主题</div>
      ) : (
        <Card>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>预览</TableHead>
                  <TableHead>主题</TableHead>
                  <TableHead>状态</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((t) => {
                  const hint = deleteDisabledHint(t)
                  return (
                    <TableRow key={t.slug}>
                      <TableCell>
                        {t.preview_url ? (
                          <img
                            src={t.preview_url}
                            alt={`${t.name} 预览`}
                            className="h-14 w-24 rounded-md border object-cover"
                          />
                        ) : (
                          <div className="flex h-14 w-24 items-center justify-center rounded-md border bg-muted text-muted-foreground">
                            <ImageIcon className="size-5" />
                          </div>
                        )}
                      </TableCell>
                      <TableCell>
                        <div className="font-medium">{t.name}</div>
                        <div className="text-xs text-muted-foreground">
                          {t.slug} · v{t.version}
                          {t.author ? ` · ${t.author}` : ""}
                        </div>
                        {t.description && (
                          <div className="mt-0.5 max-w-64 truncate text-xs text-muted-foreground">
                            {t.description}
                          </div>
                        )}
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1">
                          {t.active && <Badge>激活中</Badge>}
                          {t.builtin && <Badge variant="secondary">内置</Badge>}
                          {t.has_css && <Badge variant="outline">自定义 CSS</Badge>}
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center justify-end gap-1">
                          {/* 设置入口只服务激活主题（管理 panel 端点即激活主题） */}
                          <Button
                            variant="ghost"
                            size="sm"
                            title={t.active ? "主题设置" : "请先激活该主题，再进入其设置面板"}
                            onClick={() => {
                              if (t.active) navigate("/admin/themes/settings")
                              else toast.info(`请先激活主题「${t.name}」，再配置其设置项`)
                            }}
                          >
                            <Settings2Icon />
                            设置
                          </Button>
                          <Button
                            variant={t.active ? "ghost" : "outline"}
                            size="sm"
                            disabled={t.active || activatingSlug === t.slug}
                            onClick={() => void handleActivate(t)}
                          >
                            {activatingSlug === t.slug ? (
                              <Loader2Icon className="animate-spin" />
                            ) : t.active ? (
                              <CheckIcon />
                            ) : null}
                            {t.active ? "当前主题" : "激活"}
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label="删除"
                            title={hint ?? "删除主题"}
                            className="text-destructive hover:text-destructive"
                            disabled={hint !== null}
                            onClick={() => setDeleting(t)}
                          >
                            <Trash2Icon />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(o) => !o && setDeleting(null)}
        title="删除主题"
        description={`确定删除主题「${deleting?.name ?? ""}」吗？主题目录将被移除，此操作不可撤销。`}
        loading={deleteLoading}
        onConfirm={handleDelete}
      />
    </div>
  )
}
