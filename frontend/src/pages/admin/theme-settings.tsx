import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react"
import { Loader2Icon, SaveIcon } from "lucide-react"
import { toast } from "sonner"

import { BlockSpinner } from "@/components/spinner"
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
import { applyThemeSettingValues } from "@/lib/theme-settings"
import type { ThemeSettingDecl, ThemeSettingValue, ThemeSettingsPanel } from "@/lib/types"

// 保存错误码 → 用户文案（契约：docs/extensibility-contract.md「主题设置 API」）
const SAVE_ERR_MAP: Record<string, string> = {
  unknown_setting: "存在主题未声明的设置项，请刷新后重试（主题可能已更新）",
  invalid_value: "设置值不合法：类型不符 / select 越界 / 颜色格式错误 / 超长",
  invalid_manifest: "主题的 settings 声明非法，无法保存",
}

/** 编辑态统一用字符串（switch 用 "true"/"false"），提交时按声明类型转换 */
type Draft = Record<string, string>

const HEX6 = /^#[0-9a-fA-F]{6}$/

function toDraft(values: Record<string, ThemeSettingValue>): Draft {
  const d: Draft = {}
  for (const [k, v] of Object.entries(values)) d[k] = String(v)
  return d
}

/** 按 group 分组（保持声明顺序；分组按首次出现排序，无分组归入「通用」） */
function groupSettings(settings: ThemeSettingDecl[]): [string, ThemeSettingDecl[]][] {
  const order: string[] = []
  const byGroup = new Map<string, ThemeSettingDecl[]>()
  for (const s of settings) {
    const g = s.group?.trim() || "通用"
    if (!byGroup.has(g)) {
      byGroup.set(g, [])
      order.push(g)
    }
    byGroup.get(g)!.push(s)
  }
  return order.map((g) => [g, byGroup.get(g)!])
}

/** /admin/themes/settings：当前激活主题的设置面板（仿 Typecho「外观 → 设置」） */
export default function AdminThemeSettingsPage() {
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [panel, setPanel] = useState<ThemeSettingsPanel | null>(null)
  const [draft, setDraft] = useState<Draft>({})
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    document.title = "主题设置 · reedblog"
    api.admin
      .themeSettingsPanel()
      .then((p) => {
        setPanel(p)
        setDraft(toDraft(p.values))
      })
      .catch((e) => setLoadError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [])

  const groups = useMemo(() => groupSettings(panel?.settings ?? []), [panel])

  const setKey = (key: string, value: string) =>
    setDraft((prev) => ({ ...prev, [key]: value }))

  /** 编辑态 → PUT values（switch → bool、number → 数字；number 留空则跳过该 key） */
  const buildValues = (): Record<string, ThemeSettingValue> | null => {
    if (!panel) return null
    const out: Record<string, ThemeSettingValue> = {}
    for (const s of panel.settings) {
      const raw = (draft[s.key] ?? "").trim()
      switch (s.type) {
        case "switch":
          out[s.key] = raw === "true"
          break
        case "number": {
          if (raw === "") break // 无值的 number 不提交（保持已存值/默认值）
          const n = Number(raw)
          if (!Number.isFinite(n)) {
            toast.error(`「${s.label}」必须是数字`)
            return null
          }
          out[s.key] = n
          break
        }
        case "color": {
          if (raw === "") break
          if (!/^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(raw)) {
            toast.error(`「${s.label}」必须是 hex 颜色（如 #1a2b3c）`)
            return null
          }
          out[s.key] = raw
          break
        }
        default:
          out[s.key] = draft[s.key] ?? ""
      }
    }
    return out
  }

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!panel || saving) return
    const values = buildValues()
    if (values === null) return
    setSaving(true)
    try {
      const saved = await api.admin.updateThemeSettings(panel.slug, values)
      setPanel({ ...saved, name: panel.name })
      setDraft(toDraft(saved.values))
      // 立即应用到当前文档（CSS 变量 / data 属性 / 布局 store），无需刷新
      applyThemeSettingValues(saved.slug, saved.settings, saved.values)
      toast.success(`主题「${panel.name}」设置已保存，前台立即生效`)
    } catch (err) {
      toast.error(errorMessage(err, SAVE_ERR_MAP))
    } finally {
      setSaving(false)
    }
  }

  if (loading) return <BlockSpinner label="加载主题设置…" />
  if (loadError) {
    return (
      <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
        {loadError}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-bold">主题设置</h1>
        <p className="text-sm text-muted-foreground">
          当前激活主题：{panel?.name}（{panel?.slug}）。设置项由主题 theme.toml 的
          [[settings]] 声明，按主题独立保存，切换主题互不影响。
        </p>
      </div>

      {groups.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            该主题无自定义设置项
          </CardContent>
        </Card>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-6">
          {groups.map(([group, items]) => (
            <Card key={group}>
              <CardHeader>
                <CardTitle className="text-base">{group}</CardTitle>
                <CardDescription>共 {items.length} 项设置</CardDescription>
              </CardHeader>
              <CardContent className="grid gap-5">
                {items.map((s) => (
                  <SettingControl
                    key={s.key}
                    decl={s}
                    value={draft[s.key] ?? ""}
                    onChange={(v) => setKey(s.key, v)}
                  />
                ))}
              </CardContent>
            </Card>
          ))}

          <div>
            <Button type="submit" disabled={saving}>
              {saving ? (
                <Loader2Icon className="size-4 animate-spin" />
              ) : (
                <SaveIcon className="size-4" />
              )}
              保存设置
            </Button>
          </div>
        </form>
      )}
    </div>
  )
}

/** 单个设置项控件（按声明 type 分派；编辑态统一字符串） */
function SettingControl({
  decl,
  value,
  onChange,
}: {
  decl: ThemeSettingDecl
  value: string
  onChange: (v: string) => void
}) {
  const id = `theme-setting-${decl.key}`
  const hint =
    decl.default !== undefined && decl.default !== null
      ? `默认：${String(decl.default)}`
      : undefined

  let control: ReactNode
  switch (decl.type) {
    case "textarea":
      control = (
        <Textarea
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          rows={3}
          maxLength={5000}
        />
      )
      break
    case "color":
      control = (
        <div className="flex items-center gap-2">
          {/* 原生 color input 只认 #rrggbb；非法/短格式时拾色器显示黑色，文本框仍是权威值 */}
          <input
            type="color"
            aria-label={`${decl.label} 拾色器`}
            value={HEX6.test(value.trim()) ? value.trim() : "#000000"}
            onChange={(e) => onChange(e.target.value)}
            className="h-9 w-12 shrink-0 cursor-pointer rounded-md border bg-background p-1"
          />
          <Input
            id={id}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            placeholder="#1a2b3c"
            maxLength={9}
            className="max-w-40 font-mono"
          />
        </div>
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
    default:
      control = (
        <Input
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          maxLength={500}
          className="max-w-sm"
        />
      )
  }

  return (
    <div className="grid gap-2">
      <Label htmlFor={decl.type === "switch" ? undefined : id} className="flex items-center gap-2">
        {decl.label}
        <code className="rounded bg-muted px-1 py-0.5 text-[11px] font-normal text-muted-foreground">
          {decl.key}
        </code>
      </Label>
      {control}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}
