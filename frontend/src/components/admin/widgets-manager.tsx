import { useCallback, useEffect, useMemo, useState } from "react"
import {
  ChevronDownIcon,
  ChevronUpIcon,
  Loader2Icon,
  PlusIcon,
  SaveIcon,
  Trash2Icon,
} from "lucide-react"
import { toast } from "sonner"

import { BlockSpinner } from "@/components/spinner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
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
import { api, errorMessage } from "@/lib/api"
import { useThemeSettings } from "@/lib/theme-settings"
import type {
  ThemeSettingDecl,
  WidgetAdminItem,
  WidgetConfigValues,
  WidgetInput,
  WidgetPosition,
} from "@/lib/types"
import { cn } from "@/lib/utils"
import { loadWidgets, POSITION_LABELS, regionsForLayout } from "@/lib/widgets"

// 保存错误码 → 用户文案（契约「主题组件」）
const SAVE_ERR_MAP: Record<string, string> = {
  unknown_widget: "存在未注册的内置组件 key，请刷新后重试",
  invalid_value: "组件配置不合法：position/key 非法、参数类型不符或超长",
  validation_error: "请求体格式错误（widgets 必须是数组）",
}

const SOURCE_LABELS: Record<WidgetAdminItem["source"], string> = {
  builtin: "内置",
  theme: "主题",
  admin: "自定义",
}

/** 编辑态：把生效 config 的值统一转成字符串（switch 用 "true"/"false"），提交时按类型还原 */
type ConfigDraft = Record<string, string>

interface RowState {
  key: string
  kind: WidgetAdminItem["kind"]
  label: string
  source: WidgetAdminItem["source"]
  enabled: boolean
  position: WidgetPosition
  params: ThemeSettingDecl[]
  draft: ConfigDraft
  /** 该来源是否可删除（仅后台自建的 custom 组件） */
  deletable: boolean
}

function toDraft(config: WidgetConfigValues): ConfigDraft {
  const d: ConfigDraft = {}
  for (const [k, v] of Object.entries(config)) d[k] = typeof v === "string" ? v : String(v)
  return d
}

function toRows(items: WidgetAdminItem[]): RowState[] {
  return [...items]
    .sort((a, b) => a.sort_order - b.sort_order || a.key.localeCompare(b.key))
    .map((w) => ({
      key: w.key,
      kind: w.kind,
      label: w.label,
      source: w.source,
      enabled: w.enabled,
      position: w.position,
      params: w.params ?? [],
      draft: toDraft(w.config ?? {}),
      deletable: w.kind === "custom" && w.source === "admin",
    }))
}

/** 由名称生成合法且唯一的 custom 组件 key（^[a-z0-9][a-z0-9_-]{0,63}$） */
function makeCustomKey(name: string, existing: Set<string>): string {
  let base = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
  if (!base || !/^[a-z0-9]/.test(base)) base = `box-${base}`.replace(/-+/g, "-")
  let key = `custom-${base}`.slice(0, 64)
  let i = 2
  while (existing.has(key)) key = `custom-${base}-${i++}`.slice(0, 64)
  return key
}

/** draft（字符串）→ 提交用 config（按声明类型还原 bool/number，其余字符串） */
function buildConfig(row: RowState): WidgetConfigValues {
  const out: WidgetConfigValues = {}
  // 声明参数按类型还原
  for (const p of row.params) {
    const raw = (row.draft[p.key] ?? "").trim()
    if (p.type === "switch") {
      out[p.key] = raw === "true"
    } else if (p.type === "number") {
      if (raw === "") continue
      const n = Number(raw)
      if (Number.isFinite(n)) out[p.key] = n
    } else {
      out[p.key] = row.draft[p.key] ?? ""
    }
  }
  // custom 组件的 html / 自建 title 不在 params 声明内，单独透传
  if (row.kind === "custom") {
    if ("html" in row.draft) out.html = row.draft.html ?? ""
    if (row.source === "admin") out.title = row.draft.title ?? ""
  }
  return out
}

/**
 * 后台「组件管理」区（仿 Typecho「外观 → 设置」）：列出全部可用组件
 * （内置 + 主题声明 + 后台自建），开关启用、上下移调整顺序、选择布局位置、
 * 展开编辑参数（条数/标题/HTML 等）；保存走 PUT 全量替换，成功后刷新前台 store 即时生效。
 */
export function WidgetsManager({ slug }: { slug: string }) {
  const { layout } = useThemeSettings()
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [rows, setRows] = useState<RowState[]>([])
  const [saving, setSaving] = useState(false)
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  const [showCreate, setShowCreate] = useState(false)

  // 新建自定义组件表单
  const [newName, setNewName] = useState("")
  const [newHtml, setNewHtml] = useState("")
  const [newPosition, setNewPosition] = useState<WidgetPosition>("sidebar")

  const regions = useMemo(() => regionsForLayout(layout), [layout])

  const load = useCallback(() => {
    setLoading(true)
    api.admin
      .themeWidgets(slug)
      .then((d) => setRows(toRows(d.widgets)))
      .catch((e) => setLoadError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [slug])

  useEffect(load, [load])

  const updateRow = (key: string, patch: Partial<RowState>) =>
    setRows((prev) => prev.map((r) => (r.key === key ? { ...r, ...patch } : r)))

  const updateDraft = (key: string, field: string, value: string) =>
    setRows((prev) =>
      prev.map((r) => (r.key === key ? { ...r, draft: { ...r.draft, [field]: value } } : r)),
    )

  const move = (index: number, dir: -1 | 1) => {
    setRows((prev) => {
      const next = [...prev]
      const target = index + dir
      if (target < 0 || target >= next.length) return prev
      ;[next[index], next[target]] = [next[target], next[index]]
      return next
    })
  }

  const removeRow = (key: string) =>
    setRows((prev) => prev.filter((r) => r.key !== key))

  const addCustom = () => {
    const name = newName.trim()
    if (!name) {
      toast.error("请填写组件名称")
      return
    }
    const existing = new Set(rows.map((r) => r.key))
    const key = makeCustomKey(name, existing)
    setRows((prev) => [
      ...prev,
      {
        key,
        kind: "custom",
        label: name,
        source: "admin",
        enabled: true,
        position: newPosition,
        params: [],
        draft: { title: name, html: newHtml },
        deletable: true,
      },
    ])
    setExpanded((e) => ({ ...e, [key]: true }))
    setNewName("")
    setNewHtml("")
    setShowCreate(false)
    toast.success("已添加自定义组件，保存后生效")
  }

  const save = async () => {
    if (saving) return
    setSaving(true)
    const payload: WidgetInput[] = rows.map((r, i) => ({
      key: r.key,
      kind: r.kind,
      enabled: r.enabled,
      position: r.position,
      sort_order: (i + 1) * 10,
      config: buildConfig(r),
    }))
    try {
      const saved = await api.admin.updateThemeWidgets(slug, payload)
      setRows(toRows(saved.widgets))
      // 刷新前台 store：公开站点侧栏/页脚即时生效，无需刷新页面
      await loadWidgets(saved.slug)
      toast.success("组件配置已保存，前台立即生效")
    } catch (e) {
      toast.error(errorMessage(e, SAVE_ERR_MAP))
    } finally {
      setSaving(false)
    }
  }

  if (loading) return <BlockSpinner label="加载组件配置…" />
  if (loadError) {
    return (
      <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
        {loadError}
      </div>
    )
  }

  // 位置下拉选项：当前布局可用区域 ∪ 该行现值（避免降级位置在下拉里丢失）
  const positionOptions = (current: WidgetPosition): WidgetPosition[] =>
    regions.includes(current) ? regions : [...regions, current]

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">组件管理</CardTitle>
        <CardDescription>
          启用/排序/摆放前台侧栏、页脚等区域的组件（内置 + 主题自带 + 自定义 HTML）。
          当前布局可用区域：{regions.map((r) => POSITION_LABELS[r]).join("、")}。保存后前台立即生效。
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-2">
          {rows.map((row, index) => {
            const open = !!expanded[row.key]
            const hasEditor = row.params.length > 0 || row.kind === "custom"
            return (
              <div key={row.key} className="rounded-lg border">
                <div className="flex items-center gap-2 px-3 py-2">
                  <Switch
                    checked={row.enabled}
                    onCheckedChange={(b) => updateRow(row.key, { enabled: b })}
                    aria-label={`启用 ${row.label}`}
                  />
                  <div className="flex min-w-0 flex-1 items-center gap-2">
                    <span
                      className={cn(
                        "truncate text-sm font-medium",
                        !row.enabled && "text-muted-foreground",
                      )}
                    >
                      {row.label}
                    </span>
                    <Badge variant="outline" className="shrink-0 text-[11px] font-normal">
                      {SOURCE_LABELS[row.source]}
                    </Badge>
                    <code className="hidden shrink-0 rounded bg-muted px-1 text-[11px] text-muted-foreground sm:inline">
                      {row.key}
                    </code>
                  </div>
                  <Select
                    value={row.position}
                    onValueChange={(v) => updateRow(row.key, { position: v as WidgetPosition })}
                  >
                    <SelectTrigger className="h-8 w-[92px]" aria-label={`${row.label} 位置`}>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {positionOptions(row.position).map((p) => (
                        <SelectItem key={p} value={p}>
                          {POSITION_LABELS[p]}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <div className="flex items-center gap-0.5">
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="size-8"
                      disabled={index === 0}
                      onClick={() => move(index, -1)}
                      aria-label="上移"
                    >
                      <ChevronUpIcon className="size-4" />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="size-8"
                      disabled={index === rows.length - 1}
                      onClick={() => move(index, 1)}
                      aria-label="下移"
                    >
                      <ChevronDownIcon className="size-4" />
                    </Button>
                  </div>
                  {hasEditor && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-8 px-2"
                      onClick={() => setExpanded((e) => ({ ...e, [row.key]: !open }))}
                    >
                      {open ? "收起" : "参数"}
                    </Button>
                  )}
                  {row.deletable && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="size-8 text-destructive hover:text-destructive"
                      onClick={() => removeRow(row.key)}
                      aria-label={`删除 ${row.label}`}
                    >
                      <Trash2Icon className="size-4" />
                    </Button>
                  )}
                </div>

                {open && hasEditor && (
                  <div className="grid gap-4 border-t px-3 py-3">
                    {/* 后台自建 custom：名称 + HTML 内容 */}
                    {row.kind === "custom" && row.source === "admin" && (
                      <div className="grid gap-2">
                        <Label>名称</Label>
                        <Input
                          value={row.draft.title ?? ""}
                          onChange={(e) => {
                            const v = e.target.value
                            updateDraft(row.key, "title", v)
                            updateRow(row.key, { label: v || row.key })
                          }}
                          maxLength={200}
                          className="max-w-sm"
                        />
                      </div>
                    )}
                    {/* 声明参数（内置 / 主题组件） */}
                    {row.params.map((p) => (
                      <ParamControl
                        key={p.key}
                        decl={p}
                        value={row.draft[p.key] ?? ""}
                        onChange={(v) => updateDraft(row.key, p.key, v)}
                      />
                    ))}
                    {/* custom 组件的 HTML（自建=内容；主题=覆盖，留空用主题文件） */}
                    {row.kind === "custom" && (
                      <div className="grid gap-2">
                        <Label htmlFor={`widget-html-${row.key}`}>
                          {row.source === "theme" ? "HTML 覆盖（留空则用主题包文件）" : "HTML 内容"}
                        </Label>
                        <Textarea
                          id={`widget-html-${row.key}`}
                          value={row.draft.html ?? ""}
                          onChange={(e) => updateDraft(row.key, "html", e.target.value)}
                          rows={5}
                          className="font-mono text-xs"
                          placeholder={
                            row.source === "theme"
                              ? "留空即使用主题包 assets/widgets/" + row.key + ".html"
                              : "<p>支持 HTML 与 <script>，仅站长可见的后台输入，前台按原样渲染</p>"
                          }
                        />
                        <p className="text-xs text-muted-foreground">
                          与插件注入同款信任模型：不转义、&lt;script&gt; 会执行，请仅填入可信内容。
                          可用 <code>{"{{参数名}}"}</code> 引用上方参数值。
                        </p>
                      </div>
                    )}
                  </div>
                )}
              </div>
            )
          })}
          {rows.length === 0 && (
            <p className="py-6 text-center text-sm text-muted-foreground">暂无组件</p>
          )}
        </div>

        {/* 新建自定义组件 */}
        {showCreate ? (
          <div className="grid gap-3 rounded-lg border bg-muted/30 p-3">
            <div className="grid gap-2 sm:grid-cols-2">
              <div className="grid gap-2">
                <Label htmlFor="new-widget-name">名称</Label>
                <Input
                  id="new-widget-name"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  placeholder="如：关于作者"
                  maxLength={200}
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="new-widget-position">位置</Label>
                <Select
                  value={newPosition}
                  onValueChange={(v) => setNewPosition(v as WidgetPosition)}
                >
                  <SelectTrigger id="new-widget-position">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {regions.map((r) => (
                      <SelectItem key={r} value={r}>
                        {POSITION_LABELS[r]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="new-widget-html">HTML 内容</Label>
              <Textarea
                id="new-widget-html"
                value={newHtml}
                onChange={(e) => setNewHtml(e.target.value)}
                rows={5}
                className="font-mono text-xs"
                placeholder="<p>任意 HTML / <script> 片段</p>"
              />
            </div>
            <div className="flex gap-2">
              <Button type="button" onClick={addCustom}>
                <PlusIcon className="size-4" />
                添加到列表
              </Button>
              <Button type="button" variant="ghost" onClick={() => setShowCreate(false)}>
                取消
              </Button>
            </div>
          </div>
        ) : (
          <Button type="button" variant="outline" onClick={() => setShowCreate(true)}>
            <PlusIcon className="size-4" />
            新建自定义组件
          </Button>
        )}

        <div>
          <Button type="button" onClick={save} disabled={saving}>
            {saving ? (
              <Loader2Icon className="size-4 animate-spin" />
            ) : (
              <SaveIcon className="size-4" />
            )}
            保存组件配置
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

/** 单个声明参数控件（按 type 分派；编辑态统一字符串） */
function ParamControl({
  decl,
  value,
  onChange,
}: {
  decl: ThemeSettingDecl
  value: string
  onChange: (v: string) => void
}) {
  const id = `widget-param-${decl.key}`
  let control: React.ReactNode
  switch (decl.type) {
    case "textarea":
      control = (
        <Textarea id={id} value={value} onChange={(e) => onChange(e.target.value)} rows={3} />
      )
      break
    case "switch":
      control = (
        <Switch
          checked={value === "true"}
          onCheckedChange={(b) => onChange(b ? "true" : "false")}
          aria-label={decl.label}
        />
      )
      break
    case "number":
      control = (
        <Input
          id={id}
          type="number"
          step="any"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="max-w-40"
        />
      )
      break
    case "select":
      control = (
        <Select value={value} onValueChange={onChange}>
          <SelectTrigger id={id} className="w-full max-w-sm">
            <SelectValue placeholder="请选择" />
          </SelectTrigger>
          <SelectContent>
            {(decl.options ?? []).map((o) => (
              <SelectItem key={o.value} value={o.value}>
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )
      break
    case "color":
      control = (
        <Input
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="#1a2b3c"
          className="max-w-40 font-mono"
        />
      )
      break
    default:
      control = (
        <Input id={id} value={value} onChange={(e) => onChange(e.target.value)} className="max-w-sm" />
      )
  }
  return (
    <div className="grid gap-2">
      <Label htmlFor={decl.type === "switch" ? undefined : id}>{decl.label}</Label>
      {control}
    </div>
  )
}
