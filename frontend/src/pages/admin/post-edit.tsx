import { useEffect, useState, type FormEvent } from "react"
import { useNavigate, useParams } from "react-router-dom"
import { ArrowLeftIcon, ExternalLinkIcon, Loader2Icon, PlusIcon, SaveIcon } from "lucide-react"
import { toast } from "sonner"

import { MarkdownEditor } from "@/components/markdown-editor"
import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { api, errorMessage } from "@/lib/api"
import { cn } from "@/lib/utils"
import type { Category, PostSaveBody, PostStatus, Tag } from "@/lib/types"

const NO_CATEGORY = "none"

export default function AdminPostEditPage() {
  const { id } = useParams()
  const isEdit = id !== undefined
  const navigate = useNavigate()

  const [loading, setLoading] = useState(isEdit)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [currentSlug, setCurrentSlug] = useState<string | null>(null)
  const [currentStatus, setCurrentStatus] = useState<PostStatus | null>(null)

  const [title, setTitle] = useState("")
  const [slug, setSlug] = useState("")
  const [content, setContent] = useState("")
  const [excerpt, setExcerpt] = useState("")
  const [categoryId, setCategoryId] = useState<string>(NO_CATEGORY)
  const [tagIds, setTagIds] = useState<number[]>([])
  const [status, setStatus] = useState<PostStatus>("draft")
  const [categories, setCategories] = useState<Category[]>([])
  const [tags, setTags] = useState<Tag[]>([])
  const [newTag, setNewTag] = useState("")
  const [creatingTag, setCreatingTag] = useState(false)
  const [saving, setSaving] = useState(false)

  // 加载分类/标签选项；编辑模式下加载文章
  useEffect(() => {
    let cancelled = false
    const loadOptions = async () => {
      try {
        const [cats, tgs] = await Promise.all([api.admin.categories(), api.admin.tags()])
        if (cancelled) return
        setCategories(cats)
        setTags(tgs)
      } catch (e) {
        if (!cancelled) toast.error(errorMessage(e))
      }
    }
    void loadOptions()

    if (isEdit) {
      setLoading(true)
      api.admin
        .post(Number(id))
        .then((p) => {
          if (cancelled) return
          setTitle(p.title)
          setSlug(p.slug)
          setContent(p.content_md)
          setExcerpt(p.excerpt ?? "")
          setCategoryId(p.category_id === null ? NO_CATEGORY : String(p.category_id))
          setTagIds(p.tag_ids)
          setStatus(p.status)
          setCurrentSlug(p.slug)
          setCurrentStatus(p.status)
        })
        .catch((e) => {
          if (!cancelled) setLoadError(errorMessage(e))
        })
        .finally(() => {
          if (!cancelled) setLoading(false)
        })
    }
    return () => {
      cancelled = true
    }
  }, [id, isEdit])

  const toggleTag = (tagId: number) => {
    setTagIds((prev) =>
      prev.includes(tagId) ? prev.filter((x) => x !== tagId) : [...prev, tagId],
    )
  }

  const handleCreateTag = async () => {
    const name = newTag.trim()
    if (!name || creatingTag) return
    setCreatingTag(true)
    try {
      const created = await api.admin.createTag(name)
      setTags((prev) => [...prev, created])
      setTagIds((prev) => (prev.includes(created.id) ? prev : [...prev, created.id]))
      setNewTag("")
      toast.success(`已创建标签「${name}」`)
    } catch (err) {
      // 已存在同名标签 → 直接选中它
      const existing = tags.find((t) => t.name === name)
      if (existing) {
        setTagIds((prev) => (prev.includes(existing.id) ? prev : [...prev, existing.id]))
        setNewTag("")
      } else {
        toast.error(errorMessage(err, { duplicate_name: "标签已存在" }))
      }
    } finally {
      setCreatingTag(false)
    }
  }

  const handleSave = async (e?: FormEvent) => {
    e?.preventDefault()
    if (saving) return
    if (!title.trim()) {
      toast.error("请填写文章标题")
      return
    }
    setSaving(true)
    const body: PostSaveBody = {
      title: title.trim(),
      ...(slug.trim() ? { slug: slug.trim() } : {}),
      content_md: content,
      ...(excerpt.trim() ? { excerpt: excerpt.trim() } : {}),
      category_id: categoryId === NO_CATEGORY ? null : Number(categoryId),
      tag_ids: tagIds,
      status,
    }
    try {
      if (isEdit) {
        await api.admin.updatePost(Number(id), body)
        toast.success("文章已保存")
      } else {
        await api.admin.createPost(body)
        toast.success(status === "published" ? "文章已发布" : "草稿已创建")
      }
      navigate("/admin/posts")
    } catch (err) {
      toast.error(
        errorMessage(err, {
          slug_taken: "Slug 已被占用，请换一个",
          validation_error: "内容校验失败，请检查填写项",
        }),
      )
    } finally {
      setSaving(false)
    }
  }

  if (loading) return <BlockSpinner label="加载文章…" />
  if (loadError) {
    return (
      <div className="flex flex-col items-center gap-4 py-16">
        <p className="text-sm text-destructive">{loadError}</p>
        <Button variant="outline" onClick={() => navigate("/admin/posts")}>
          返回文章列表
        </Button>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      {/* 顶部操作栏 */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="icon"
            aria-label="返回列表"
            onClick={() => navigate("/admin/posts")}
          >
            <ArrowLeftIcon />
          </Button>
          <h1 className="text-xl font-bold">{isEdit ? "编辑文章" : "新建文章"}</h1>
          {isEdit && currentStatus === "published" && currentSlug && (
            <Button variant="ghost" size="sm" asChild>
              <a href={`/posts/${encodeURIComponent(currentSlug)}`} target="_blank" rel="noreferrer">
                <ExternalLinkIcon />
                查看
              </a>
            </Button>
          )}
        </div>
        <div className="flex items-center gap-3">
          {/* 草稿 / 发布切换 */}
          <div className="inline-flex rounded-md border p-0.5" role="group" aria-label="发布状态">
            {(["draft", "published"] as const).map((s) => (
              <button
                key={s}
                type="button"
                onClick={() => setStatus(s)}
                className={cn(
                  "rounded px-3 py-1 text-sm transition-colors",
                  status === s
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                )}
              >
                {s === "draft" ? "草稿" : "发布"}
              </button>
            ))}
          </div>
          <Button onClick={() => void handleSave()} disabled={saving}>
            {saving ? <Loader2Icon className="animate-spin" /> : <SaveIcon />}
            保存
          </Button>
        </div>
      </div>

      {/* 标题 / slug / 摘要 */}
      <Card>
        <CardContent className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor="post-title">标题 *</Label>
            <Input
              id="post-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="文章标题"
              className="text-base"
            />
          </div>
          <div className="grid gap-1.5 sm:max-w-md">
            <Label htmlFor="post-slug">Slug</Label>
            <Input
              id="post-slug"
              value={slug}
              onChange={(e) => setSlug(e.target.value)}
              placeholder={isEdit ? undefined : "留空自动生成"}
            />
            <p className="text-xs text-muted-foreground">用于文章 URL /posts/:slug，留空自动生成</p>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="post-excerpt">摘要（选填）</Label>
            <Textarea
              id="post-excerpt"
              value={excerpt}
              onChange={(e) => setExcerpt(e.target.value)}
              placeholder="列表页展示的摘要，留空则不显示"
              className="min-h-16"
            />
          </div>
        </CardContent>
      </Card>

      {/* Markdown 编辑器（textarea + 实时预览 + 图片上传；与页面编辑共用组件） */}
      <Card>
        <CardContent className="grid gap-4">
          <MarkdownEditor id="post-content" value={content} onChange={setContent} />
        </CardContent>
      </Card>

      {/* 分类 / 标签 */}
      <Card>
        <CardContent className="grid gap-6">
          <div className="grid gap-1.5 sm:max-w-md">
            <Label>分类</Label>
            <Select value={categoryId} onValueChange={setCategoryId}>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="选择分类" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NO_CATEGORY}>无分类</SelectItem>
                {categories.map((c) => (
                  <SelectItem key={c.id} value={String(c.id)}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="grid gap-2">
            <Label>标签（点击多选）</Label>
            <div className="flex flex-wrap gap-2">
              {tags.map((t) => {
                const selected = tagIds.includes(t.id)
                return (
                  <button
                    key={t.id}
                    type="button"
                    onClick={() => toggleTag(t.id)}
                    aria-pressed={selected}
                    className={cn(
                      "inline-flex items-center rounded-md border px-2.5 py-1 text-xs transition-colors",
                      selected
                        ? "border-primary bg-primary text-primary-foreground"
                        : "bg-background hover:bg-accent",
                    )}
                  >
                    {t.name}
                  </button>
                )
              })}
              {tags.length === 0 && (
                <span className="text-sm text-muted-foreground">暂无标签，可在下方新建</span>
              )}
            </div>
            <div className="flex max-w-sm gap-2">
              <Input
                value={newTag}
                onChange={(e) => setNewTag(e.target.value)}
                placeholder="新标签名"
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault()
                    void handleCreateTag()
                  }
                }}
              />
              <Button
                type="button"
                variant="outline"
                disabled={creatingTag || !newTag.trim()}
                onClick={() => void handleCreateTag()}
              >
                {creatingTag ? <Loader2Icon className="animate-spin" /> : <PlusIcon />}
                新建
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* 底部保存 */}
      <div className="flex justify-end">
        <Button onClick={() => void handleSave()} disabled={saving}>
          {saving ? <Loader2Icon className="animate-spin" /> : <SaveIcon />}
          {isEdit ? "保存修改" : status === "published" ? "创建并发布" : "创建草稿"}
        </Button>
      </div>
    </div>
  )
}
