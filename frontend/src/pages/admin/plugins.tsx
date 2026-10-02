import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import { Loader2Icon, Trash2Icon, UploadIcon } from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
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
import type { PluginInfo } from "@/lib/types"

// 上传/启用错误码 → 用户文案（契约：docs/extensibility-contract.md 插件管理 API）
const UPLOAD_ERR_MAP: Record<string, string> = {
  plugin_exists: "已存在同 slug 的插件，请先删除旧插件或修改包内 slug",
  invalid_manifest: "manifest.toml 不合法：字段格式有误，或 hooks/inject 含未知值",
  invalid_package: "zip 包不合法：根层须为单个插件目录且包含 manifest.toml",
}

const ENABLE_ERR_MAP: Record<string, string> = {
  script_error: "Rhai 脚本语法错误，插件未启用（详见「最近错误」列）",
}

export default function AdminPluginsPage() {
  const [items, setItems] = useState<PluginInfo[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [file, setFile] = useState<File | null>(null)
  const [uploading, setUploading] = useState(false)
  const [togglingSlug, setTogglingSlug] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<PluginInfo | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  const load = useCallback(() => {
    api.admin
      .plugins()
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
      const created = await api.admin.uploadPlugin(file)
      setItems((prev) => [created, ...(prev ?? [])])
      toast.success(`插件「${created.name}」已安装（默认未启用，请手动开启）`)
      setFile(null)
      if (fileRef.current) fileRef.current.value = ""
    } catch (err) {
      toast.error(errorMessage(err, UPLOAD_ERR_MAP))
    } finally {
      setUploading(false)
    }
  }

  const toggleEnabled = async (p: PluginInfo) => {
    setTogglingSlug(p.slug)
    try {
      const updated = p.enabled
        ? await api.admin.disablePlugin(p.slug)
        : await api.admin.enablePlugin(p.slug)
      setItems((prev) => (prev ?? []).map((it) => (it.slug === updated.slug ? updated : it)))
      toast.success(updated.enabled ? `插件「${updated.name}」已启用` : `插件「${updated.name}」已停用`)
    } catch (err) {
      toast.error(errorMessage(err, ENABLE_ERR_MAP))
      load() // 失败后重新拉取，保证开关与服务端状态一致
    } finally {
      setTogglingSlug(null)
    }
  }

  const handleDelete = async () => {
    if (!deleting) return
    setDeleteLoading(true)
    try {
      await api.admin.deletePlugin(deleting.slug)
      setItems((prev) => (prev ?? []).filter((it) => it.slug !== deleting.slug))
      toast.success(`插件「${deleting.name}」已删除`)
      setDeleting(null)
    } catch (err) {
      toast.error(errorMessage(err))
      setDeleting(null)
    } finally {
      setDeleteLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-bold">插件管理</h1>
        <p className="text-sm text-muted-foreground">
          共 {items?.length ?? 0} 个插件；上传 zip 安装（根层为单个插件目录，含 manifest.toml），
          安装后默认停用，需手动启用。
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
              上传安装
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
        <div className="py-12 text-center text-sm text-muted-foreground">
          暂无插件，上传 zip 安装第一个插件
        </div>
      ) : (
        <Card>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>插件</TableHead>
                  <TableHead>钩子</TableHead>
                  <TableHead>注入</TableHead>
                  <TableHead>最近错误</TableHead>
                  <TableHead>启用</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((p) => (
                  <TableRow key={p.slug}>
                    <TableCell>
                      <div className="font-medium">{p.name}</div>
                      <div className="text-xs text-muted-foreground">
                        {p.slug} · v{p.version}
                        {p.author ? ` · ${p.author}` : ""}
                      </div>
                      {p.description && (
                        <div className="mt-0.5 max-w-56 truncate text-xs text-muted-foreground">
                          {p.description}
                        </div>
                      )}
                    </TableCell>
                    <TableCell className="max-w-48">
                      <div className="flex flex-wrap gap-1">
                        {(p.hooks ?? []).length === 0 ? (
                          <span className="text-xs text-muted-foreground">—</span>
                        ) : (
                          (p.hooks ?? []).map((h) => (
                            <Badge key={h} variant="outline" className="font-mono text-xs">
                              {h}
                            </Badge>
                          ))
                        )}
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {(p.inject ?? []).length === 0 ? (
                          <span className="text-xs text-muted-foreground">—</span>
                        ) : (
                          (p.inject ?? []).map((pos) => (
                            <Badge key={pos} variant="secondary" className="font-mono text-xs">
                              {pos}
                            </Badge>
                          ))
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="max-w-44">
                      {p.last_error ? (
                        <span className="block truncate text-xs text-destructive" title={p.last_error}>
                          {p.last_error}
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground">—</span>
                      )}
                    </TableCell>
                    <TableCell>
                      {togglingSlug === p.slug ? (
                        <Loader2Icon className="size-4 animate-spin text-muted-foreground" />
                      ) : (
                        <Switch
                          checked={p.enabled}
                          aria-label={`${p.enabled ? "停用" : "启用"}插件 ${p.name}`}
                          onCheckedChange={() => void toggleEnabled(p)}
                        />
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="删除"
                          className="text-destructive hover:text-destructive"
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
        title="删除插件"
        description={`确定删除插件「${deleting?.name ?? ""}」吗？插件目录将被移除，此操作不可撤销。`}
        loading={deleteLoading}
        onConfirm={handleDelete}
      />
    </div>
  )
}
