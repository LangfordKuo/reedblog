import { useSyncExternalStore } from "react"

/**
 * 文章目录（TOC）全局 store + 标题提取（纯前端，契约零改动）：
 * - 文章详情页正文渲染完成后调用 extractTocHeadings() 扫描标题元素，
 *   为无 id 的标题生成稳定 id（slug 化文本 + 去重后缀）写回 DOM，
 *   产物 {id, text, level}[] 经 setTocHeadings() 写入 store；
 * - <PostToc /> 与布局壳（三列右栏 / 双列页面侧栏）通过 useTocHeadings()
 *   订阅——TOC 是文章详情页固有部件，不走 widgets 配置，但需要跨组件树
 *   共享（三列布局的右栏由 SiteLayout 渲染，页面无法直接塞入）；
 * - 离开详情页（卸载/切换文章）时清空，右栏 TOC 随之消失。
 */

export interface TocHeading {
  /** 标题元素 id（锚点目标；提取时已保证页面内唯一） */
  id: string
  /** 标题纯文本（textContent trim） */
  text: string
  /** 标题级别（2=h2，3=h3，4=h4） */
  level: number
}

/** 标题数少于该阈值时整个 TOC 不渲染（需求：< 2 不显示） */
export const TOC_MIN_HEADINGS = 2

/** 默认提取深度：h2/h3；改为 4 即额外收录 h4（需求「可配置到 h4」） */
export const TOC_MAX_LEVEL = 3

/** 锚点落点偏移（px）：写为标题元素的 scroll-margin-top，覆盖 sticky header（56~64px） */
export const HEADING_SCROLL_MARGIN_PX = 80

/** 高亮判定线（px）：标题顶部滚过该线即视为「当前阅读位置」（略低于锚点落点，跳转后即选中） */
export const TOC_ACTIVE_LINE_PX = 88

let state: TocHeading[] = []
const listeners = new Set<() => void>()

function emit() {
  for (const l of listeners) l()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => void listeners.delete(listener)
}

/** 组件订阅入口（PostToc / 布局壳判断右栏是否渲染） */
export function useTocHeadings(): TocHeading[] {
  return useSyncExternalStore(subscribe, () => state)
}

/** 写入当前文章的标题列表（[] = 无 TOC） */
export function setTocHeadings(headings: TocHeading[]): void {
  state = headings
  emit()
}

/** 是否达到 TOC 渲染门槛（组件与布局壳共用同一判断，避免空壳） */
export function shouldRenderToc(headings: TocHeading[]): boolean {
  return headings.length >= TOC_MIN_HEADINGS
}

/**
 * 标题文本 → slug：小写，字母/数字（含中文等 Unicode 字母）以外的
 * 连续字符折叠为连字符，去首尾连字符；空结果回退 "section"。
 * 例："Hello, World!" → "hello-world"；「一杯手冲的时间」原样保留。
 */
export function slugifyHeadingText(text: string): string {
  const s = text
    .toLowerCase()
    .replace(/[^\p{Letter}\p{Number}]+/gu, "-")
    .replace(/^-+|-+$/g, "")
  return s || "section"
}

/**
 * 标题纯文本：KaTeX 会同时输出视觉隐藏的 MathML 副本（`.katex-mathml`，含
 * `annotation` 里的 LaTeX 源码），直接取 textContent 会把源码与可见排版重复
 * 混进 TOC；有公式时按副本移除后再取文本，普通标题零开销。
 */
function headingText(el: HTMLElement): string {
  if (!el.querySelector(".katex-mathml")) return (el.textContent ?? "").trim()
  const clone = el.cloneNode(true) as HTMLElement
  clone.querySelectorAll(".katex-mathml").forEach((n) => n.remove())
  return (clone.textContent ?? "").trim()
}

/**
 * 从正文容器提取 h2..h{maxLevel} 标题（文档顺序）：
 * - 元素已有 id 直接沿用；无 id 时按 slugifyHeadingText 生成，
 *   与本次已收集的 id 及页面现存元素 id 冲突时追加 -1/-2… 后缀，写回 el.id；
 * - 统一为标题元素写 scroll-margin-top（原生 #hash 跳转与 scrollIntoView 同享偏移）；
 * - 过滤空文本标题。
 * 对 react-markdown 产物与后端 content_html（dangerouslySetInnerHTML）同样适用。
 */
export function extractTocHeadings(
  root: HTMLElement,
  maxLevel: number = TOC_MAX_LEVEL,
): TocHeading[] {
  const selector = Array.from({ length: maxLevel - 1 }, (_, i) => `h${i + 2}`).join(",")
  const used = new Set<string>()
  const out: TocHeading[] = []
  for (const el of Array.from(root.querySelectorAll<HTMLElement>(selector))) {
    const text = headingText(el)
    if (!text) continue
    let id = el.id
    if (id) {
      used.add(id)
    } else {
      const base = slugifyHeadingText(text)
      let candidate = base
      for (let n = 1; used.has(candidate) || document.getElementById(candidate); n += 1) {
        candidate = `${base}-${n}`
      }
      id = candidate
      el.id = id
      used.add(id)
    }
    el.style.scrollMarginTop = `${HEADING_SCROLL_MARGIN_PX}px`
    out.push({ id, text, level: Number(el.tagName.slice(1)) })
  }
  return out
}
