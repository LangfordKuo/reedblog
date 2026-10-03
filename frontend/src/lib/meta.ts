/**
 * 运行时 head 元信息注入（契约「SEO / 分享元信息」条款，2026-10-04 新增）。
 *
 * 爬虫/社媒预览不执行 JS，它们的 OG HTML 由后端按 UA 直接返回（见后端 seo 模块 /
 * deploy/nginx.conf.example）；本模块负责**真人 SPA** 侧的运行时 meta：
 * document.title、meta[name=description]、og:title/og:description/og:url、link[rel=canonical]。
 *
 * 与插件注入系统（head 片段）互不打架的策略：本模块创建/接管的标签统一带
 * `data-reedblog-meta="1"` 标注；更新按 name/property/rel 精确匹配（插件已注入的
 * 同名标签会被接管并标注），只有本模块标注过且不再需要的标签才会被移除。
 */

import { loadSiteSettings } from "./site"

/** 本模块管理标签的统一标注属性（便于覆盖/清理，与插件注入区分） */
const MARKER = "data-reedblog-meta"

/** 创建或更新 `<meta name|property="key">`（命中的既有标签被接管并打标注） */
export function setMeta(
  attr: "name" | "property",
  key: string,
  content: string,
): HTMLMetaElement {
  let el = document.head.querySelector<HTMLMetaElement>(`meta[${attr}="${key}"]`)
  if (!el) {
    el = document.createElement("meta")
    el.setAttribute(attr, key)
    document.head.appendChild(el)
  }
  el.setAttribute("content", content)
  el.setAttribute(MARKER, "1")
  return el
}

/** 移除由本模块接管的 `<meta>`（不存在或由插件管理则不动） */
export function removeMeta(attr: "name" | "property", key: string): void {
  document.head.querySelector<HTMLMetaElement>(`meta[${attr}="${key}"][${MARKER}]`)?.remove()
}

/** 创建或更新 `<link rel="canonical">` */
export function setCanonical(href: string): void {
  let el = document.head.querySelector<HTMLLinkElement>('link[rel="canonical"]')
  if (!el) {
    el = document.createElement("link")
    el.setAttribute("rel", "canonical")
    document.head.appendChild(el)
  }
  el.setAttribute("href", href)
  el.setAttribute(MARKER, "1")
}

/** 当前页绝对 URL（origin + SPA 路径；base_url 设置只影响后端 OG HTML，前端用实际访问地址） */
function absoluteUrl(path: string): string {
  return new URL(path, window.location.origin).toString()
}

export interface PageMeta {
  /** 页面标题（不含站点名）；缺省时标题即站点名（首页） */
  title?: string
  /** meta description；缺省回退站点设置 description */
  description?: string
  /** SPA 路径（如 /posts/hello），canonical 与 og:url 用 */
  path: string
  /** og:type，默认 website */
  type?: "website" | "article"
}

// 页面切换竞态防护：异步补站点名时只允许最后一次 applyPageMeta 生效
let seq = 0

/**
 * 应用页面级元信息：同步写入 title、描述、og 各项与 canonical，
 * 站点设置到位后再补全「标题 - 站点名」、og:site_name 与站点描述兜底。
 * 站点设置请求失败时保留同步阶段的兜底值，不阻塞页面。
 */
export function applyPageMeta(meta: PageMeta): void {
  const my = ++seq
  const url = absoluteUrl(meta.path)
  const title = meta.title?.trim() ?? ""

  // 同步兜底（站点名未知）：标题不含站点名、描述只有调用方显式给出时才写
  document.title = title || "reedblog"
  if (meta.description?.trim()) setMeta("name", "description", meta.description.trim())
  setMeta("property", "og:type", meta.type ?? "website")
  if (title) setMeta("property", "og:title", title)
  setMeta("property", "og:url", url)
  setCanonical(url)

  loadSiteSettings()
    .then((s) => {
      if (my !== seq) return
      const siteTitle = s.title.trim()
      document.title = title
        ? siteTitle
          ? `${title} - ${siteTitle}`
          : title
        : siteTitle || "reedblog"
      setMeta("property", "og:title", title || siteTitle)
      if (siteTitle) setMeta("property", "og:site_name", siteTitle)
      const description = meta.description?.trim() || s.description.trim()
      if (description) {
        setMeta("name", "description", description)
        setMeta("property", "og:description", description)
      }
    })
    .catch(() => {
      /* 站点设置拉取失败：保留同步兜底 */
    })
}
