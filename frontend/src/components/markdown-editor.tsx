import { useEffect, useRef, useState, type ClipboardEvent, type DragEvent } from "react"
import { ImageIcon, Loader2Icon } from "lucide-react"
import { toast } from "sonner"

import { Markdown } from "@/components/markdown"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { api, errorMessage } from "@/lib/api"
import { cn } from "@/lib/utils"

/** 上传接口的错误码 → 用户文案（其余错误用服务端 message） */
const UPLOAD_ERROR_MESSAGES: Record<string, string> = {
  invalid_file_type: "仅支持 PNG / JPEG / GIF / WebP 图片（SVG 不允许）",
  file_too_large: "图片超出服务器大小上限",
  validation_error: "上传请求格式有误",
}

export interface MarkdownEditorProps {
  value: string
  onChange: (value: string) => void
  /** textarea 的 id（配 Label htmlFor） */
  id?: string
  label?: string
  placeholder?: string
  /** 编辑区/预览区最小高度类（默认 min-h-[28rem]） */
  minHeightClass?: string
}

/**
 * Markdown 编辑器：textarea + 实时预览分栏，支持图片上传
 * （工具栏选择 / 粘贴 / 拖拽，上传后在光标处插入 ![filename](url)）。
 * 文章编辑与页面编辑共用（契约「页面」条款：页面编辑器复用文章编辑器组件）。
 */
export function MarkdownEditor({
  value,
  onChange,
  id = "md-editor-content",
  label = "正文（Markdown）",
  placeholder = "使用 Markdown 写作，支持 GFM 语法（表格、任务列表、删除线等）；可粘贴或拖拽图片自动上传…",
  minHeightClass = "min-h-[28rem]",
}: MarkdownEditorProps) {
  const contentRef = useRef<HTMLTextAreaElement>(null)
  const imageInputRef = useRef<HTMLInputElement>(null)
  const [uploading, setUploading] = useState(0)
  // 内容变化后要把光标放回插入点之后（受控组件在重渲染时才会应用新值）
  const pendingCaret = useRef<number | null>(null)

  useEffect(() => {
    if (pendingCaret.current !== null && contentRef.current) {
      const pos = pendingCaret.current
      pendingCaret.current = null
      contentRef.current.focus()
      contentRef.current.setSelectionRange(pos, pos)
    }
  }, [value])

  /** 在光标处插入文本（无焦点/拿不到光标时追加到末尾） */
  const insertAtCursor = (text: string) => {
    const ta = contentRef.current
    const start = ta ? Math.min(ta.selectionStart, value.length) : value.length
    const end = ta ? Math.min(ta.selectionEnd, value.length) : value.length
    pendingCaret.current = start + text.length
    onChange(value.slice(0, start) + text + value.slice(end))
  }

  /** 逐个上传图片并在光标处插入 ![filename](url)；失败 toast 提示 */
  const uploadImages = async (files: File[]) => {
    for (const file of files) {
      setUploading((n) => n + 1)
      try {
        const res = await api.admin.uploadImage(file)
        // alt 文本里剔除会破坏 Markdown 图片语法的字符
        const alt = res.filename.replace(/[[\]()]/g, "")
        insertAtCursor(`![${alt}](${res.url})\n`)
      } catch (err) {
        toast.error(`「${file.name}」上传失败：${errorMessage(err, UPLOAD_ERROR_MESSAGES)}`)
      } finally {
        setUploading((n) => n - 1)
      }
    }
  }

  /** 从 FileList 过滤出图片后上传（工具栏选择 / 拖拽共用） */
  const uploadFromFiles = (fileList: FileList | null | undefined) => {
    if (!fileList || fileList.length === 0) return
    const images = Array.from(fileList).filter((f) => f.type.startsWith("image/"))
    if (images.length === 0) {
      toast.error("仅支持 PNG / JPEG / GIF / WebP 图片（SVG 不允许）")
      return
    }
    void uploadImages(images)
  }

  /** 粘贴剪贴板图片：有图片则拦截默认粘贴，自动上传 */
  const handlePaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = e.clipboardData?.files
    if (!files || files.length === 0) return
    const images = Array.from(files).filter((f) => f.type.startsWith("image/"))
    if (images.length === 0) return
    e.preventDefault()
    void uploadImages(images)
  }

  /** 拖拽图片文件到正文：拦截默认行为（否则浏览器会直接打开文件），自动上传 */
  const handleDrop = (e: DragEvent<HTMLTextAreaElement>) => {
    e.preventDefault()
    uploadFromFiles(e.dataTransfer?.files)
  }

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <div className="grid content-start gap-1.5">
        <div className="flex items-center justify-between gap-2">
          <Label htmlFor={id}>{label}</Label>
          <div className="flex items-center gap-2">
            {uploading > 0 && (
              <span className="text-xs text-muted-foreground">图片上传中…（{uploading}）</span>
            )}
            <input
              ref={imageInputRef}
              type="file"
              accept="image/png,image/jpeg,image/gif,image/webp"
              multiple
              className="hidden"
              onChange={(e) => {
                uploadFromFiles(e.target.files)
                // 清空 value：同一文件再次选择也能触发 change
                e.target.value = ""
              }}
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={uploading > 0}
              onClick={() => imageInputRef.current?.click()}
            >
              {uploading > 0 ? <Loader2Icon className="animate-spin" /> : <ImageIcon />}
              插入图片
            </Button>
          </div>
        </div>
        <Textarea
          id={id}
          ref={contentRef}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onPaste={handlePaste}
          onDragOver={(e) => e.preventDefault()}
          onDrop={handleDrop}
          placeholder={placeholder}
          className={cn("flex-1 resize-y font-mono text-sm leading-relaxed", minHeightClass)}
        />
      </div>
      <div className="grid content-start gap-1.5">
        <Label>实时预览</Label>
        <div className={cn("overflow-auto rounded-md border bg-card p-4", minHeightClass)}>
          {value.trim() ? (
            <Markdown>{value}</Markdown>
          ) : (
            <p className="text-sm text-muted-foreground">预览区域，左侧输入后实时渲染。</p>
          )}
        </div>
      </div>
    </div>
  )
}
