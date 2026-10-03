import { api } from "./api"
import { loadThemeSettings } from "./theme-settings"
import type { ThemeTokens } from "./types"

// 已知令牌 key 集合（下划线形式，对应 index.css 的 shadcn 变量体系）。
// 契约规定：未知 key 忽略（向前兼容）；缺失 key 不写内联变量，
// 自然回落到 index.css :root / .dark 的内置默认值。
const KNOWN_TOKEN_KEYS = new Set([
  "radius",
  "background",
  "foreground",
  "card",
  "card_foreground",
  "popover",
  "popover_foreground",
  "primary",
  "primary_foreground",
  "secondary",
  "secondary_foreground",
  "muted",
  "muted_foreground",
  "accent",
  "accent_foreground",
  "destructive",
  "border",
  "input",
  "ring",
  "chart_1",
  "chart_2",
  "chart_3",
  "chart_4",
  "chart_5",
  "sidebar",
  "sidebar_foreground",
  "sidebar_primary",
  "sidebar_primary_foreground",
  "sidebar_accent",
  "sidebar_accent_foreground",
  "sidebar_border",
  "sidebar_ring",
])

const DARK_STYLE_ID = "reedblog-theme-dark"
const CSS_LINK_ID = "reedblog-theme-css"

/** 令牌 key（下划线）→ CSS 变量名（中划线），如 primary_foreground → --primary-foreground */
function cssVarName(key: string): string {
  return `--${key.replace(/_/g, "-")}`
}

/**
 * 令牌值 → CSS 值：
 * - radius 原样写入（契约规定值自带单位，如 0.5rem）
 * - 契约为 HSL 分量串（如 "240 10% 3.9%"，不带 hsl() 包裹），补全为 hsl(...)
 * - 已是完整颜色函数 / hex / var() 的值原样写入（兼容主题直接给 oklch 等）
 */
function tokenValue(key: string, raw: string): string {
  const v = raw.trim()
  if (key === "radius") return v
  if (/^(hsl|rgb|hwb|lab|lch|oklab|oklch|color|color-mix|light-dark|var)\(|^#/i.test(v)) return v
  return `hsl(${v})`
}

function usableEntries(tokens: ThemeTokens | null | undefined): [string, string][] {
  if (!tokens) return []
  return Object.entries(tokens).filter(
    (e): e is [string, string] =>
      KNOWN_TOKEN_KEYS.has(e[0]) && typeof e[1] === "string" && e[1].trim() !== "",
  )
}

/** 亮色令牌写入 :root（documentElement 内联样式，优先级高于 index.css） */
function applyTokens(tokens: ThemeTokens | null | undefined): void {
  const root = document.documentElement
  // 先清掉上一次应用的全部已知变量，避免热切换主题时旧值残留、
  // 并让新主题缺失的 key 回落到 index.css 内置默认
  for (const key of KNOWN_TOKEN_KEYS) root.style.removeProperty(cssVarName(key))
  for (const [key, raw] of usableEntries(tokens)) {
    root.style.setProperty(cssVarName(key), tokenValue(key, raw))
  }
}

/**
 * 暗色令牌走 index.css 既有的 .dark 类机制：注入一段 `.dark { ... }` 样式。
 * 必须加 !important：亮色令牌写在 documentElement 内联样式上，
 * 而内联声明优先于普通样式表规则，只有 important 作者声明能压过它。
 */
function applyDarkTokens(tokens: ThemeTokens | null | undefined): void {
  document.getElementById(DARK_STYLE_ID)?.remove()
  const entries = usableEntries(tokens)
  if (entries.length === 0) return
  const decls = entries.map(([key, raw]) => `${cssVarName(key)}: ${tokenValue(key, raw)} !important;`)
  const style = document.createElement("style")
  style.id = DARK_STYLE_ID
  style.textContent = `.dark { ${decls.join(" ")} }`
  document.head.appendChild(style)
}

/** theme.css 以 <link rel=stylesheet> 注入 head（css_url 为 null 表示主题无自定义 CSS） */
function applyThemeCss(cssUrl: string | null | undefined): void {
  document.getElementById(CSS_LINK_ID)?.remove()
  if (!cssUrl) return
  const link = document.createElement("link")
  link.id = CSS_LINK_ID
  link.rel = "stylesheet"
  link.href = cssUrl
  document.head.appendChild(link)
}

/**
 * 拉取激活主题并应用（公开站点与后台共用，启动时调用；后台切换主题后可再次调用热切换）。
 * GET /api/themes/active 在未安装门禁白名单内，未安装时返回 default 主题，安装页同样有样式。
 * 接口不可用（后端未就绪/网络错误）时静默跳过，保持 index.css 内置默认样式。
 *
 * 令牌之后叠加应用主题设置（契约「主题设置项-前端应用约定」）：
 * 生效值写入 --theme-setting-* CSS 变量与 data-setting-* / data-layout 属性，
 * 并更新 theme-settings store 供布局组件订阅；设置端点失败不影响令牌管线。
 */
export async function applyActiveTheme(): Promise<string | null> {
  try {
    const theme = await api.themeActive()
    applyTokens(theme.tokens)
    applyDarkTokens(theme.tokens_dark)
    applyThemeCss(theme.css_url)
    await loadThemeSettings(theme.slug)
    return theme.slug
  } catch {
    return null
  }
}
