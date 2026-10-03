import { useCallback, useEffect, useRef, useState } from "react"
import { useSearchParams } from "react-router-dom"
import { CopyIcon, ImageIcon, ImageOffIcon, Loader2Icon, Trash2Icon, UploadIcon } from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Pagination } from "@/components/pagination"
import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { api, errorMessage } from "@/lib/api"
import { formatBytes, formatDateTime } from "@/lib/utils"
import type { MediaItem, Page } from "@/lib/types"

const PER_PAGE = 12

const ACCEPT = "image/png,image/jpeg,image/gif,image/webp"

/** 媒体库（契约「媒体库」条款）：网格展示 + 上传 / 复制 URL / 删除（含引用提示） */
export default function AdminMediaPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const page = Math.max(1, Number(searchParams.get("page")) || 1)

  const [data, setData] = useState<Page<MediaItem> | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)
  const [deleting, setDeleting] = useState<MediaItem | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const load = useCallback(() => {
    setLoading(true)
    api.admin
      .media({ page, per_page: PER_PAGE })
      .then((d) => {
        setData(d)
        setError(null)
      })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [page])

  useEffect(load, [load])

  const goFirstPage = () => {
    const next = new URLSearchParams(searchParams)
    next.delete("page")
    setSearchParams(next, { replace: true })
  }

  const handleUpload = async (files: FileList | null) => {
    if (!files || files.length === 0) return
    const list = Array.from(files)
    setUploading(true)
    let ok = 0
    try {
      for (const file of list) {
        try {
          await api.admin.uploadImage(file)
          ok += 1
        } catch (e) {
          toast.error(`${file.name}：${errorMessage(e)}`)
        }
      }
      if (ok > 0) {
        toast.success(ok === 1 ? "图片已上传" : `已上传 ${ok} 张图片`)
        // 新图按 created_at DESC 排在最前：回到第 1 页再刷新
        if (page !== 1) goFirstPage()
        else load()
      }
    } finally {
      setUploading(false)
      if (fileInputRef.current) fileInputRef.current.value = ""
    }
  }

  const copyUrl = async (item: MediaItem) => {
    try {
      await navigator.clipboard.writeText(item.url)
      toast.success("图片 URL 已复制")
    } catch {
      toast.error("复制失败，请手动复制")
    }
  }

  const handleDelete = async () => {
    if (!deleting) return
    setDeleteLoading(true)
    try {
      await api.admin.deleteMedia(deleting.id)
      toast.success("图片已删除")
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
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">媒体库</h1>
          <p className="text-sm text-muted-foreground">共 {data?.total ?? 0} 张图片</p>
        </div>
        <div>
          <input
            ref={fileInputRef}
            type="file"
            accept={ACCEPT}
            multiple
            className="hidden"
            onChange={(e) => void handleUpload(e.target.files)}
          />
          <Button disabled={uploading} onClick={() => fileInputRef.current?.click()}>
            {uploading ? <Loader2Icon className="animate-spin" /> : <UploadIcon />}
            上传图片
          </Button>
        </div>
      </div>

      {loading ? (
        <BlockSpinner />
      ) : error ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
          {error}
        </div>
      ) : data && data.items.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-12 text-center">
            <ImageIcon className="size-8 text-muted-foreground" />
            <p className="text-sm text-muted-foreground">还没有上传图片</p>
            <Button
              variant="outline"
              disabled={uploading}
              onClick={() => fileInputRef.current?.click()}
            >
              <UploadIcon />
              上传第一张图片
            </Button>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {data?.items.map((item) => (
            <Card key={item.id} className="gap-0 overflow-hidden py-0">
              {/* 缩略图直接以 url 为 img src；加载失败（文件被外部清掉等）时留占位图标 */}
              <div className="relative flex h-40 items-center justify-center border-b bg-muted">
                <ImageOffIcon className="size-6 text-muted-foreground" />
                <img
                  src={item.url}
                  alt={item.filename}
                  loading="lazy"
                  className="absolute inset-0 size-full object-contain"
                  onError={(e) => {
                    e.currentTarget.style.display = "none"
                  }}
                />
              </div>
              <CardContent className="grid gap-0.5 p-3">
                <div className="truncate font-medium" title={item.filename}>
                  {item.filename}
                </div>
                <div className="text-xs text-muted-foreground">
                  {item.width && item.height ? `${item.width} × ${item.height}` : "尺寸未知"}
                  {" · "}
                  {formatBytes(item.size)}
                </div>
                <div className="text-xs text-muted-foreground">
                  {formatDateTime(item.created_at)}
                </div>
                <div className="mt-2 flex justify-end gap-1">
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="复制 URL"
                    title="复制 URL"
                    onClick={() => void copyUrl(item)}
                  >
                    <CopyIcon />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="删除"
                    title="删除"
                    className="text-destructive hover:text-destructive"
                    onClick={() => setDeleting(item)}
                  >
                    <Trash2Icon />
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {data && (
        <Pagination
          page={data.page}
          perPage={data.per_page}
          total={data.total}
          onChange={(p) => {
            const next = new URLSearchParams(searchParams)
            if (p <= 1) next.delete("page")
            else next.set("page", String(p))
            setSearchParams(next, { replace: true })
          }}
        />
      )}

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(o) => !o && setDeleting(null)}
        title="删除图片"
        description={`确定删除「${deleting?.filename ?? ""}」吗？该图片可能已被文章引用，删除后旧文章里的该图会 404。此操作不可撤销。`}
        loading={deleteLoading}
        onConfirm={handleDelete}
      />
    </div>
  )
}
