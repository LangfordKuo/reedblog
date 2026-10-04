import { useEffect, useMemo, useState } from "react"
import { useNavigate, useParams } from "react-router-dom"
import {
  ArrowDownIcon,
  ArrowLeftIcon,
  ArrowUpIcon,
  ExternalLinkIcon,
  Link2Icon,
  Loader2Icon,
  PlusIcon,
  SaveIcon,
  Trash2Icon,
} from "lucide-react"
import { toast } from "sonner"

import { DraftRestoreDialog } from "@/components/admin/draft-restore-dialog"
import { MarkdownEditor } from "@/components/markdown-editor"
import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import { useAutosaveDraft } from "@/hooks/use-autosave-draft"
import { api, errorMessage } from "@/lib/api"
import {
  clearDraft,
  draftsEqual,
  pageDraftKey,
  readDraft,
  type DraftEnvelope,
  type PageDraftData,
} from "@/lib/draft"
import { ICON_NAMES, getPageIcon } from "@/lib/page-icons"
import type { PageAdmin, PageKind, PageLinkBody, PageSaveBody } from "@/lib/types"

const KIND_LABELS: Record<PageKind, string> = {
  custom: "普通页",
  message_board: "留言板",
  links: "友情链接",
}

/**
 * 「不设置（用默认）」选项的哨兵值：Radix Select 禁止 value=""，
 * 而合法图标名只能是 [a-z0-9-]（契约「页面-图标」），下划线哨兵永不与之冲突
 */
const ICON_NONE = "__none__"
const ICON_NONE_LABEL = "不设置（用默认）"

/** 新建页面的空白草稿（基线 + 草稿比对基准） */
const EMPTY_PAGE_DRAFT: PageDraftData = {
  title: "",
  slug: "",
  content: "",
  icon: "",
  sortOrder: "0",
  links: [],
}

/** 服务端页面 → 草稿快照（与编辑器受控字段一致；links 顺序即排序，保持原序） */
function toDraftSnapshot(p: PageAdmin): PageDraftData {
  return {
    title: p.title,
    slug: p.slug,
    content: p.content_md,
    icon: p.icon,
    sortOrder: String(p.sort_order),
    links: p.links.map((l) => ({ name: l.name, url: l.url, description: l.description })),
  }
}

/** 新建/编辑页面（契约「页面」条款）：
 * 标题、slug、Markdown 内容（复用文章编辑器组件）、排序；
 * kind=links 页面额外渲染友情链接管理区（名称/URL/描述/排序，增删改，全量替换保存）。
 * kind 与 built_in 不可改，仅展示。 */
export default function AdminPageEditPage() {
  const { id } = useParams()
  const isEdit = id !== undefined
  const navigate = useNavigate()

  const [loading, setLoading] = useState(isEdit)
  const [loadError, setLoadError] = useState<string | null>(null)

  const [title, setTitle] = useState("")
  const [slug, setSlug] = useState("")
  const [content, setContent] = useState("")
  // 图标名（空串=不设置，前台按 kind/slug 回退默认；契约「页面-图标」）
  const [icon, setIcon] = useState("")
  const [sortOrder, setSortOrder] = useState("0")
  const [kind, setKind] = useState<PageKind>("custom")
  const [builtIn, setBuiltIn] = useState(false)
  const [enabled, setEnabled] = useState(true)
  const [currentSlug, setCurrentSlug] = useState<string | null>(null)
  // 友情链接（仅 kind=links 生效；数组顺序即排序，保存时全量替换）
  const [links, setLinks] = useState<PageLinkBody[]>([])
  const [saving, setSaving] = useState(false)
  // 本地草稿（防丢稿）：serverDraft 为服务端基线（null=尚未加载完成，不写草稿）
  const [serverDraft, setServerDraft] = useState<PageDraftData | null>(
    isEdit ? null : EMPTY_PAGE_DRAFT,
  )
  const [pendingDraft, setPendingDraft] = useState<DraftEnvelope<PageDraftData> | null>(null)

  const draftStorageKey = pageDraftKey(isEdit ? Number(id) : "new")

  useEffect(() => {
    if (!isEdit) return
    let cancelled = false
    setLoading(true)
    api.admin
      .page(Number(id))
      .then((p) => {
        if (cancelled) return
        setTitle(p.title)
        setSlug(p.slug)
        setContent(p.content_md)
        setIcon(p.icon)
        setSortOrder(String(p.sort_order))
        setKind(p.kind)
        setBuiltIn(p.built_in)
        setEnabled(p.enabled)
        setCurrentSlug(p.slug)
        setLinks(p.links.map((l) => ({ name: l.name, url: l.url, description: l.description })))
        // 本地草稿：与服务端版本不同才提示恢复（相同则直接清理）
        const snapshot = toDraftSnapshot(p)
        setServerDraft(snapshot)
        const key = pageDraftKey(Number(id))
        const stored = readDraft<PageDraftData>(key)
        if (stored) {
          if (draftsEqual(stored.data, snapshot)) clearDraft(key)
          else setPendingDraft(stored)
        }
      })
      .catch((e) => {
        if (!cancelled) setLoadError(errorMessage(e))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [id, isEdit])

  // 新建页面：进入页面即检查本地草稿（存在且非空白才提示恢复）
  useEffect(() => {
    if (isEdit) return
    const key = pageDraftKey("new")
    const stored = readDraft<PageDraftData>(key)
    if (!stored) return
    if (draftsEqual(stored.data, EMPTY_PAGE_DRAFT)) clearDraft(key)
    else setPendingDraft(stored)
  }, [isEdit])

  // 当前编辑器快照 → 自动保存（1.5s 防抖 + 30s 强制；页面隐藏/卸载时补落）
  const draftSnapshot = useMemo<PageDraftData>(
    () => ({ title, slug, content, icon, sortOrder, links }),
    [title, slug, content, icon, sortOrder, links],
  )
  const { markSaved } = useAutosaveDraft({
    storageKey: draftStorageKey,
    data: draftSnapshot,
    baseline: serverDraft,
    enabled: !loading && !loadError,
  })

  /** 恢复草稿：字段回填（含友情链接） */
  const handleRestoreDraft = () => {
    const d = pendingDraft?.data
    if (!d) return
    setTitle(d.title)
    setSlug(d.slug)
    setContent(d.content)
    // 旧版草稿（v1，无 icon 字段）恢复时按「不设置」处理，不静默丢内容
    setIcon(d.icon ?? "")
    setSortOrder(d.sortOrder)
    setLinks(d.links.map((l) => ({ name: l.name, url: l.url, description: l.description })))
    setPendingDraft(null)
    toast.success("已恢复本地草稿，确认无误后请点击保存")
  }

  const handleDiscardDraft = () => {
    clearDraft(draftStorageKey)
    setPendingDraft(null)
  }

  const updateLink = (idx: number, patch: Partial<PageLinkBody>) => {
    setLinks((prev) => prev.map((l, i) => (i === idx ? { ...l, ...patch } : l)))
  }

  const moveLink = (idx: number, dir: -1 | 1) => {
    setLinks((prev) => {
      const to = idx + dir
      if (to < 0 || to >= prev.length) return prev
      const next = [...prev]
      ;[next[idx], next[to]] = [next[to], next[idx]]
      return next
    })
  }

  const handleSave = async () => {
    if (saving) return
    if (!title.trim()) {
      toast.error("请填写页面标题")
      return
    }
    // 链接轻校验（后端同款规则会再校验一次）
    if (kind === "links") {
      for (const l of links) {
        if (!l.name.trim() || !l.url.trim()) {
          toast.error("友情链接的名称与 URL 不能为空")
          return
        }
      }
    }
    setSaving(true)
    const body: PageSaveBody = {
      title: title.trim(),
      ...(slug.trim() ? { slug: slug.trim() } : {}),
      content_md: content,
      // 空串=清除（前台回退 kind 默认）；后端只校验格式，不枚举名字
      icon,
      sort_order: Number(sortOrder) || 0,
      // 全量替换语义；仅 links 页提交（其余 kind 后端也会忽略）
      ...(kind === "links"
        ? {
            links: links.map((l) => ({
              name: l.name.trim(),
              url: l.url.trim(),
              description: l.description?.trim() || undefined,
            })),
          }
        : {}),
    }
    try {
      if (isEdit) {
        await api.admin.updatePage(Number(id), body)
        toast.success("页面已保存")
      } else {
        await api.admin.createPage(body)
        toast.success("页面已创建")
      }
      // 后端已确认保存：清除本地草稿（须在跳转卸载前，防止卸载补落把草稿写回）
      markSaved()
      navigate("/admin/pages")
    } catch (err) {
      toast.error(
        errorMessage(err, {
          slug_taken: "Slug 已被其他页面占用，请换一个",
          validation_error: "内容校验失败，请检查填写项",
        }),
      )
    } finally {
      setSaving(false)
    }
  }

  // 图标选项：库里已存但不在精选表的合法名字（如 API 直接写入的）也列出，
  // 避免用户一保存就被静默改写；预览走 getPageIcon（未知名字即 kind 默认）
  const iconOptions: string[] =
    icon && !(ICON_NAMES as readonly string[]).includes(icon)
      ? [icon, ...ICON_NAMES]
      : [...ICON_NAMES]

  if (loading) return <BlockSpinner label="加载页面…" />
  if (loadError) {
    return (
      <div className="flex flex-col items-center gap-4 py-16">
        <p className="text-sm text-destructive">{loadError}</p>
        <Button variant="outline" onClick={() => navigate("/admin/pages")}>
          返回页面列表
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
            onClick={() => navigate("/admin/pages")}
          >
            <ArrowLeftIcon />
          </Button>
          <h1 className="text-xl font-bold">{isEdit ? "编辑页面" : "新建页面"}</h1>
          {isEdit && (
            <Badge variant="secondary">{KIND_LABELS[kind]}</Badge>
          )}
          {builtIn && <Badge variant="outline">内置</Badge>}
          {isEdit && enabled && currentSlug && (
            <Button variant="ghost" size="sm" asChild>
              <a
                href={`/pages/${encodeURIComponent(currentSlug)}`}
                target="_blank"
                rel="noreferrer"
              >
                <ExternalLinkIcon />
                查看
              </a>
            </Button>
          )}
        </div>
        <Button onClick={() => void handleSave()} disabled={saving}>
          {saving ? <Loader2Icon className="animate-spin" /> : <SaveIcon />}
          保存
        </Button>
      </div>

      {/* 标题 / slug / 排序 / 图标 */}
      <Card>
        <CardContent className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor="page-title">标题 *</Label>
            <Input
              id="page-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="页面标题（如：关于、留言板）"
              className="text-base"
            />
          </div>
          <div className="grid gap-1.5 sm:max-w-md">
            <Label htmlFor="page-slug">Slug</Label>
            <Input
              id="page-slug"
              value={slug}
              onChange={(e) => setSlug(e.target.value)}
              placeholder={isEdit ? undefined : "留空自动生成"}
            />
            <p className="text-xs text-muted-foreground">
              用于页面 URL /pages/:slug，留空自动生成；页面间唯一
            </p>
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <Label htmlFor="page-sort">排序</Label>
              <Input
                id="page-sort"
                type="number"
                value={sortOrder}
                onChange={(e) => setSortOrder(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                数值越小越靠前（导航与列表按此排序；内置页依次为 10 / 20 / 30）
              </p>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="page-icon">图标</Label>
              <Select
                value={icon || ICON_NONE}
                onValueChange={(v) => setIcon(v === ICON_NONE ? "" : v)}
              >
                <SelectTrigger id="page-icon" className="w-full">
                  <SelectValue placeholder={ICON_NONE_LABEL} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ICON_NONE}>{ICON_NONE_LABEL}</SelectItem>
                  {iconOptions.map((name) => {
                    const Icon = getPageIcon(name, kind, slug)
                    return (
                      <SelectItem key={name} value={name}>
                        <span className="flex items-center gap-2">
                          <Icon className="size-4 text-muted-foreground" aria-hidden />
                          {name}
                        </span>
                      </SelectItem>
                    )
                  })}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                前台导航与标题旁显示；不设置时按页面类型自动选择（留言板/友情链接/关于各有默认）
              </p>
            </div>
          </div>
          {isEdit && kind !== "custom" && (
            <p className="text-xs text-muted-foreground">
              页面类型（{KIND_LABELS[kind]}）创建后不可修改
              {builtIn ? "；内置页面不可删除，但可停用、可编辑内容" : ""}
            </p>
          )}
        </CardContent>
      </Card>

      {/* Markdown 编辑器（与文章编辑共用组件） */}
      <Card>
        <CardContent className="grid gap-4">
          <MarkdownEditor id="page-content" value={content} onChange={setContent} />
        </CardContent>
      </Card>

      {/* 友情链接管理区（仅 kind=links 页面渲染） */}
      {kind === "links" && (
        <Card>
          <CardContent className="grid gap-4">
            <div className="flex items-center justify-between gap-2">
              <div>
                <Label className="flex items-center gap-1.5">
                  <Link2Icon className="size-4" />
                  友情链接
                </Label>
                <p className="mt-1 text-xs text-muted-foreground">
                  按数组顺序展示（用 ↑ ↓ 调整排序）；保存时全量替换该页链接
                </p>
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => setLinks((prev) => [...prev, { name: "", url: "", description: "" }])}
              >
                <PlusIcon />
                添加链接
              </Button>
            </div>
            <Separator />
            {links.length === 0 ? (
              <p className="py-4 text-center text-sm text-muted-foreground">
                暂无链接，点击「添加链接」新增
              </p>
            ) : (
              <div className="grid gap-3">
                {links.map((l, idx) => (
                  <div key={idx} className="grid gap-2 rounded-lg border p-3">
                    <div className="grid gap-2 sm:grid-cols-2">
                      <div className="grid gap-1">
                        <Label className="text-xs text-muted-foreground">名称 *</Label>
                        <Input
                          value={l.name}
                          onChange={(e) => updateLink(idx, { name: e.target.value })}
                          placeholder="站点名称"
                          maxLength={100}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label className="text-xs text-muted-foreground">URL *</Label>
                        <Input
                          value={l.url}
                          onChange={(e) => updateLink(idx, { url: e.target.value })}
                          placeholder="https://example.com"
                          maxLength={500}
                        />
                      </div>
                    </div>
                    <div className="grid gap-1">
                      <Label className="text-xs text-muted-foreground">描述（选填）</Label>
                      <Input
                        value={l.description ?? ""}
                        onChange={(e) => updateLink(idx, { description: e.target.value })}
                        placeholder="一句话介绍这个站点"
                        maxLength={500}
                      />
                    </div>
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label="上移"
                        disabled={idx === 0}
                        onClick={() => moveLink(idx, -1)}
                      >
                        <ArrowUpIcon />
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label="下移"
                        disabled={idx === links.length - 1}
                        onClick={() => moveLink(idx, 1)}
                      >
                        <ArrowDownIcon />
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label="删除链接"
                        className="text-destructive hover:text-destructive"
                        onClick={() => setLinks((prev) => prev.filter((_, i) => i !== idx))}
                      >
                        <Trash2Icon />
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* 底部保存 */}
      <div className="flex justify-end">
        <Button onClick={() => void handleSave()} disabled={saving}>
          {saving ? <Loader2Icon className="animate-spin" /> : <SaveIcon />}
          {isEdit ? "保存修改" : "创建页面"}
        </Button>
      </div>

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
