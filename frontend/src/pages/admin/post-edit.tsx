import { useEffect, useMemo, useState, type FormEvent } from "react"
import { useNavigate, useParams } from "react-router-dom"
import {
  ArrowLeftIcon,
  ExternalLinkIcon,
  HistoryIcon,
  Loader2Icon,
  PlusIcon,
  SaveIcon,
} from "lucide-react"
import { toast } from "sonner"

import { DraftRestoreDialog } from "@/components/admin/draft-restore-dialog"
import { RevisionHistoryDialog } from "@/components/admin/revision-history"
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
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { useAutosaveDraft } from "@/hooks/use-autosave-draft"
import { api, errorMessage } from "@/lib/api"
import {
  clearDraft,
  draftsEqual,
  postDraftKey,
  readDraft,
  type DraftEnvelope,
  type PostDraftData,
} from "@/lib/draft"
import { cn } from "@/lib/utils"
import type { Category, PostAdmin, PostSaveBody, PostStatus, Tag } from "@/lib/types"

const NO_CATEGORY = "none"

/** 新建文章的空白草稿（基线 + 草稿比对基准） */
const EMPTY_POST_DRAFT: PostDraftData = {
  title: "",
  slug: "",
  content: "",
  excerpt: "",
  categoryId: NO_CATEGORY,
  tagIds: [],
  status: "draft",
  isSticky: false,
  scheduleLocal: "",
}

/** 发布状态选项（scheduled=定时发布，契约「文章置顶与定时发布」条款） */
const STATUS_OPTIONS: { value: PostStatus; label: string }[] = [
  { value: "draft", label: "草稿" },
  { value: "published", label: "发布" },
  { value: "scheduled", label: "定时发布" },
]

/** RFC3339 UTC → datetime-local 输入值（本地时区 YYYY-MM-DDTHH:mm） */
function utcToLocalInput(ts: string | null): string {
  if (!ts) return ""
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return ""
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

/** datetime-local 输入值（本地时区）→ RFC3339 UTC（后端再归一化为秒精度存储） */
function localInputToUtc(value: string): string | null {
  if (!value) return null
  const d = new Date(value)
  if (Number.isNaN(d.getTime())) return null
  return d.toISOString()
}

/** 服务端文章 → 草稿快照（tagIds 排序，保证选择顺序不同也能比较相等） */
function toDraftSnapshot(p: PostAdmin): PostDraftData {
  return {
    title: p.title,
    slug: p.slug,
    content: p.content_md,
    excerpt: p.excerpt ?? "",
    categoryId: p.category_id === null ? NO_CATEGORY : String(p.category_id),
    tagIds: [...p.tag_ids].sort((a, b) => a - b),
    status: p.status,
    isSticky: p.is_sticky,
    scheduleLocal: p.status === "scheduled" ? utcToLocalInput(p.published_at) : "",
  }
}

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
  // 置顶开关（契约「文章置顶与定时发布」条款）
  const [isSticky, setIsSticky] = useState(false)
  // 定时发布时间（datetime-local 输入值，本地时区；保存时转 UTC）
  const [scheduleLocal, setScheduleLocal] = useState("")
  const [categories, setCategories] = useState<Category[]>([])
  const [tags, setTags] = useState<Tag[]>([])
  /** 分类/标签选项是否已加载完成（草稿恢复时据此决定要不要校验 id 有效性） */
  const [optionsLoaded, setOptionsLoaded] = useState(false)
  const [newTag, setNewTag] = useState("")
  const [creatingTag, setCreatingTag] = useState(false)
  const [saving, setSaving] = useState(false)
  // 修订历史对话框（仅编辑模式；契约「文章修订历史」条款）
  const [historyOpen, setHistoryOpen] = useState(false)
  // 本地草稿（防丢稿）：serverDraft 为服务端基线（null=尚未加载完成，不写草稿），
  // pendingDraft 为待用户决定的「恢复 / 丢弃」提示
  const [serverDraft, setServerDraft] = useState<PostDraftData | null>(
    isEdit ? null : EMPTY_POST_DRAFT,
  )
  const [pendingDraft, setPendingDraft] = useState<DraftEnvelope<PostDraftData> | null>(null)

  const draftStorageKey = postDraftKey(isEdit ? Number(id) : "new")

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
      } finally {
        if (!cancelled) setOptionsLoaded(true)
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
          setIsSticky(p.is_sticky)
          // scheduled 文章回显计划时间（UTC → 本地时区输入值）
          setScheduleLocal(p.status === "scheduled" ? utcToLocalInput(p.published_at) : "")
          setCurrentSlug(p.slug)
          setCurrentStatus(p.status)
          // 本地草稿：与服务端版本不同才提示恢复（相同则说明没有未保存的改动，直接清理）
          const snapshot = toDraftSnapshot(p)
          setServerDraft(snapshot)
          const stored = readDraft<PostDraftData>(postDraftKey(Number(id)))
          if (stored) {
            if (draftsEqual(stored.data, snapshot)) clearDraft(postDraftKey(Number(id)))
            else setPendingDraft(stored)
          }
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

  // 新建文章：进入页面即检查本地草稿（存在且非空白才提示恢复）
  useEffect(() => {
    if (isEdit) return
    const key = postDraftKey("new")
    const stored = readDraft<PostDraftData>(key)
    if (!stored) return
    if (draftsEqual(stored.data, EMPTY_POST_DRAFT)) clearDraft(key)
    else setPendingDraft(stored)
  }, [isEdit])

  // 当前编辑器快照 → 自动保存（1.5s 防抖 + 30s 强制；页面隐藏/卸载时补落）
  const draftSnapshot = useMemo<PostDraftData>(
    () => ({
      title,
      slug,
      content,
      excerpt,
      categoryId,
      tagIds: [...tagIds].sort((a, b) => a - b),
      status,
      isSticky,
      scheduleLocal,
    }),
    [title, slug, content, excerpt, categoryId, tagIds, status, isSticky, scheduleLocal],
  )
  const { markSaved } = useAutosaveDraft({
    storageKey: draftStorageKey,
    data: draftSnapshot,
    baseline: serverDraft,
    enabled: !loading && !loadError,
  })

  /** 恢复草稿：字段回填。选项已加载时校验分类/标签是否仍存在（避免提交出 422）；
   *  选项尚未加载（慢网络）时原样回填，绝不在不知情的情况下丢数据 */
  const handleRestoreDraft = () => {
    const d = pendingDraft?.data
    if (!d) return
    setTitle(d.title)
    setSlug(d.slug)
    setContent(d.content)
    setExcerpt(d.excerpt)
    setCategoryId(
      !optionsLoaded ||
        d.categoryId === NO_CATEGORY ||
        categories.some((c) => String(c.id) === d.categoryId)
        ? d.categoryId
        : NO_CATEGORY,
    )
    setTagIds(optionsLoaded ? d.tagIds.filter((tid) => tags.some((t) => t.id === tid)) : d.tagIds)
    setStatus(d.status)
    setIsSticky(d.isSticky)
    setScheduleLocal(d.scheduleLocal)
    setPendingDraft(null)
    toast.success("已恢复本地草稿，确认无误后请点击保存")
  }

  const handleDiscardDraft = () => {
    clearDraft(draftStorageKey)
    setPendingDraft(null)
  }

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
    const body: PostSaveBody = {
      title: title.trim(),
      ...(slug.trim() ? { slug: slug.trim() } : {}),
      content_md: content,
      ...(excerpt.trim() ? { excerpt: excerpt.trim() } : {}),
      category_id: categoryId === NO_CATEGORY ? null : Number(categoryId),
      tag_ids: tagIds,
      status,
      is_sticky: isSticky,
    }
    // 定时发布：计划时间必填且须为未来时间（本地时区输入 → UTC 存储；与后端 422 校验同款规则，
    // 前端先行拦截给出即时提示）
    if (status === "scheduled") {
      const iso = localInputToUtc(scheduleLocal)
      if (!iso) {
        toast.error("定时发布必须选择发布时间")
        return
      }
      if (new Date(iso).getTime() <= Date.now()) {
        toast.error("定时发布时间必须晚于当前时间，请使用未来时间")
        return
      }
      body.published_at = iso
    }
    setSaving(true)
    try {
      if (isEdit) {
        await api.admin.updatePost(Number(id), body)
        toast.success(
          status === "scheduled" ? "已保存，将按计划时间自动发布" : "文章已保存",
        )
      } else {
        await api.admin.createPost(body)
        toast.success(
          status === "published"
            ? "文章已发布"
            : status === "scheduled"
              ? "已创建定时发布，到点自动公开可见"
              : "草稿已创建",
        )
      }
      // 后端已确认保存：清除本地草稿（须在跳转卸载前，防止卸载补落把草稿写回）
      markSaved()
      navigate("/admin/posts")
    } catch (err) {
      // validation_error 不再整体覆盖：后端消息已足够可读（如「定时发布时间必须晚于当前时间」），
      // 透传具体原因
      toast.error(errorMessage(err, { slug_taken: "Slug 已被占用，请换一个" }))
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
          {isEdit && (
            <Button variant="ghost" size="sm" onClick={() => setHistoryOpen(true)}>
              <HistoryIcon />
              修订历史
            </Button>
          )}
        </div>
        <div className="flex items-center gap-3">
          {/* 草稿 / 发布 / 定时发布切换 */}
          <div className="inline-flex rounded-md border p-0.5" role="group" aria-label="发布状态">
            {STATUS_OPTIONS.map((opt) => {
              // 后端拒绝 published→scheduled（契约：先转草稿），此处直接置灰提示
              const disabled = opt.value === "scheduled" && currentStatus === "published"
              return (
                <button
                  key={opt.value}
                  type="button"
                  disabled={disabled}
                  title={
                    disabled ? "已发布文章不能直接改为定时发布，请先转为草稿" : undefined
                  }
                  onClick={() => {
                    setStatus(opt.value)
                    // 首次切到定时发布：默认预填「1 小时后」（本地时区显示）
                    if (opt.value === "scheduled" && !scheduleLocal) {
                      setScheduleLocal(
                        utcToLocalInput(new Date(Date.now() + 3600_000).toISOString()),
                      )
                    }
                  }}
                  className={cn(
                    "rounded px-3 py-1 text-sm transition-colors",
                    status === opt.value
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                    disabled &&
                      "cursor-not-allowed opacity-40 hover:bg-transparent hover:text-muted-foreground",
                  )}
                >
                  {opt.label}
                </button>
              )
            })}
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
          {/* 定时发布时间（status=scheduled 时必填；本地时区输入、保存转 UTC） */}
          {status === "scheduled" && (
            <div className="grid gap-1.5 sm:max-w-md">
              <Label htmlFor="post-schedule">定时发布时间 *（本地时区）</Label>
              <Input
                id="post-schedule"
                type="datetime-local"
                value={scheduleLocal}
                onChange={(e) => setScheduleLocal(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                到点后自动公开可见（无需重启）；保存时转换为 UTC，必须晚于当前时间
              </p>
            </div>
          )}
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

          {/* 置顶开关（契约「文章置顶与定时发布」条款） */}
          <div className="flex items-center justify-between gap-4 border-t pt-5 sm:max-w-md">
            <div className="grid gap-0.5">
              <Label>置顶文章</Label>
              <p className="text-xs text-muted-foreground">
                前台列表排在最前并显示「置顶」徽章；RSS/sitemap/搜索排序不受影响
              </p>
            </div>
            <Switch
              aria-label="置顶文章"
              checked={isSticky}
              onCheckedChange={setIsSticky}
            />
          </div>
        </CardContent>
      </Card>

      {/* 底部保存 */}
      <div className="flex justify-end">
        <Button onClick={() => void handleSave()} disabled={saving}>
          {saving ? <Loader2Icon className="animate-spin" /> : <SaveIcon />}
          {isEdit
            ? "保存修改"
            : status === "published"
              ? "创建并发布"
              : status === "scheduled"
                ? "创建定时发布"
                : "创建草稿"}
        </Button>
      </div>

      {/* 修订历史（恢复后回填编辑器；差别对比的是当前编辑中的正文，未保存修改也参与） */}
      {isEdit && (
        <RevisionHistoryDialog
          postId={Number(id)}
          currentContent={content}
          open={historyOpen}
          onOpenChange={setHistoryOpen}
          onRestored={(p) => {
            setTitle(p.title)
            setContent(p.content_md)
            setExcerpt(p.excerpt ?? "")
          }}
        />
      )}

      {/* 本地草稿恢复提示（恢复 / 丢弃由用户决定，绝不静默覆盖服务器内容） */}
      {pendingDraft && (
        <DraftRestoreDialog
          open
          savedAt={pendingDraft.savedAt}
          onOpenChange={(open) => {
            // 关闭（X/Esc/遮罩）= 稍后决定：草稿保留，下次进入仍会提示
            if (!open) setPendingDraft(null)
          }}
          onRestore={handleRestoreDraft}
          onDiscard={handleDiscardDraft}
        />
      )}
    </div>
  )
}
