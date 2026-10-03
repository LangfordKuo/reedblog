/**
 * 编辑器本地草稿（localStorage）：防浏览器崩溃 / 误关标签丢稿。
 *
 * 设计要点：
 * - key 按类型与 id 区分：`reedblog:draft:post:<id>` / `reedblog:draft:page:<id>`，
 *   新建（尚无 id）用 `post:new` / `page:new` 独立 key，多篇文章/多个标签互不干扰。
 * - 信封结构 `{ v, savedAt, data }`：v 为结构版本，读取时校验；savedAt 为写入时刻。
 * - 单条超过 DRAFT_MAX_CHARS 字符不存（避免一条把 localStorage 配额吃光）；
 *   索引（DRAFT_INDEX_KEY）记录写入顺序，超过 DRAFT_MAX_ENTRIES 条时从最旧开始清理。
 * - 所有 localStorage 访问都包在 try/catch 里：隐私模式 / 配额满 / 被禁用时
 *   静默降级（读返回 null，写返回 false），不抛错、不阻塞编辑。
 * - 纯函数（pruneDraftIndex / draftsEqual / canonicalJson / formatDraftAge）与
 *   Storage 解耦，便于单测；storage 参数可注入替身。
 */

import type { PageLinkBody, PostStatus } from "./types"

/** 键前缀（draftKey 之外不要直接使用） */
export const DRAFT_KEY_PREFIX = "reedblog:draft:"
/** 草稿索引键：记录 { key, savedAt }，用于超量清理 */
export const DRAFT_INDEX_KEY = "reedblog:draft-index"
/** 信封结构版本 */
export const DRAFT_VERSION = 1
/** 单条草稿字符数上限（localStorage 以 UTF-16 计，1M 字符 ≈ 2MB，已足够长文） */
export const DRAFT_MAX_CHARS = 1_000_000
/** 本地保留的草稿条数上限（超过则清理最旧的） */
export const DRAFT_MAX_ENTRIES = 20
/** 输入防抖：停止输入 1.5 秒后落盘 */
export const DRAFT_DEBOUNCE_MS = 1500
/** 强制落盘间隔：持续输入时最长 30 秒必落一次 */
export const DRAFT_FORCE_MS = 30_000

export type DraftKind = "post" | "page"

/** 文章编辑器的草稿载荷（与 post-edit 各受控字段一一对应） */
export interface PostDraftData {
  title: string
  slug: string
  content: string
  excerpt: string
  categoryId: string
  tagIds: number[]
  status: PostStatus
  isSticky: boolean
  scheduleLocal: string
}

/** 页面编辑器的草稿载荷（含友情链接，kind=links 时一并防丢） */
export interface PageDraftData {
  title: string
  slug: string
  content: string
  sortOrder: string
  links: PageLinkBody[]
}

export interface DraftEnvelope<T> {
  v: number
  savedAt: number
  data: T
}

/** 最小 Storage 接口（只用到这三个方法；测试可注入内存替身） */
export interface DraftStorage {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
  removeItem(key: string): void
}

/** 草稿 key：`reedblog:draft:<kind>:<id|new>` */
export function draftKey(kind: DraftKind, id: number | "new"): string {
  return `${DRAFT_KEY_PREFIX}${kind}:${id}`
}

export function postDraftKey(id: number | "new"): string {
  return draftKey("post", id)
}

export function pageDraftKey(id: number | "new"): string {
  return draftKey("page", id)
}

/** 删除文章/页面（含回收站彻底删除）时清理对应草稿 */
export function clearPostDraft(id: number | "new", storage?: DraftStorage): void {
  clearDraft(postDraftKey(id), storage)
}

export function clearPageDraft(id: number | "new", storage?: DraftStorage): void {
  clearDraft(pageDraftKey(id), storage)
}

/** localStorage 可用性探测（结果缓存；隐私模式下 setItem 可能抛错） */
let storageResolved: DraftStorage | null | undefined
function resolveStorage(): DraftStorage | null {
  if (storageResolved !== undefined) return storageResolved
  try {
    const s = globalThis.localStorage
    const probe = "reedblog:draft-probe"
    s.setItem(probe, "1")
    s.removeItem(probe)
    storageResolved = s
  } catch {
    storageResolved = null
  }
  return storageResolved
}

/** 读取草稿；不存在 / 解析失败 / 结构不符（含版本不符）均返回 null，绝不抛错 */
export function readDraft<T>(key: string, storage?: DraftStorage | null): DraftEnvelope<T> | null {
  const s = storage === undefined ? resolveStorage() : storage
  if (!s) return null
  try {
    const raw = s.getItem(key)
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== "object" || parsed === null) return null
    const env = parsed as Partial<DraftEnvelope<T>>
    if (env.v !== DRAFT_VERSION) return null
    if (typeof env.savedAt !== "number" || !Number.isFinite(env.savedAt)) return null
    if (env.data === undefined) return null
    return { v: DRAFT_VERSION, savedAt: env.savedAt, data: env.data }
  } catch {
    return null
  }
}

/**
 * 写入草稿：单条超限 / 序列化失败 / 配额满（隐私模式、localStorage 满）一律
 * 静默返回 false，不抛错；成功后顺带维护索引并清理超量旧草稿。
 */
export function writeDraft<T>(
  key: string,
  data: T,
  now: number = Date.now(),
  storage?: DraftStorage | null,
): boolean {
  const s = storage === undefined ? resolveStorage() : storage
  if (!s) return false
  let raw: string
  try {
    raw = JSON.stringify({ v: DRAFT_VERSION, savedAt: now, data })
  } catch {
    return false
  }
  if (raw.length > DRAFT_MAX_CHARS) return false
  try {
    s.setItem(key, raw)
  } catch {
    return false
  }
  touchDraftIndex(s, key, now)
  return true
}

/** 清除草稿并从索引移除；任何存储异常都静默吞掉 */
export function clearDraft(key: string, storage?: DraftStorage | null): void {
  const s = storage === undefined ? resolveStorage() : storage
  if (!s) return
  try {
    s.removeItem(key)
  } catch {
    /* 忽略：清不掉也不影响编辑 */
  }
  try {
    const entries = parseIndex(s.getItem(DRAFT_INDEX_KEY)).filter((e) => e.key !== key)
    s.setItem(DRAFT_INDEX_KEY, JSON.stringify(entries))
  } catch {
    /* 索引维护失败不影响草稿清理本身 */
  }
}

interface DraftIndexEntry {
  key: string
  savedAt: number
}

function parseIndex(raw: string | null): DraftIndexEntry[] {
  if (!raw) return []
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (e): e is DraftIndexEntry =>
        typeof e === "object" &&
        e !== null &&
        typeof (e as DraftIndexEntry).key === "string" &&
        typeof (e as DraftIndexEntry).savedAt === "number",
    )
  } catch {
    return []
  }
}

/** 写入成功后维护索引：同 key 去重（保留最新），按 savedAt 升序保留最近 max 条 */
function touchDraftIndex(s: DraftStorage, key: string, savedAt: number): void {
  try {
    const entries = parseIndex(s.getItem(DRAFT_INDEX_KEY))
    entries.push({ key, savedAt })
    const { kept, removed } = pruneDraftIndex(entries)
    // 刚写入的 key 永远保留（防时钟回拨把它算成最旧）
    for (const k of removed) {
      if (k === key) continue
      try {
        s.removeItem(k)
      } catch {
        /* 单条清理失败继续 */
      }
    }
    s.setItem(DRAFT_INDEX_KEY, JSON.stringify(kept))
  } catch {
    /* 索引维护失败不影响草稿本身 */
  }
}

/**
 * 索引清理（纯函数）：去重后按 savedAt 升序，保留最新 max 条，其余为待删除 key。
 * 传入顺序无要求；同 key 多条时以 savedAt 最大者为准。
 */
export function pruneDraftIndex(
  entries: DraftIndexEntry[],
  max: number = DRAFT_MAX_ENTRIES,
): { kept: DraftIndexEntry[]; removed: string[] } {
  const latest = new Map<string, number>()
  for (const e of entries) {
    const prev = latest.get(e.key)
    if (prev === undefined || e.savedAt > prev) latest.set(e.key, e.savedAt)
  }
  const sorted = [...latest.entries()]
    .map(([key, savedAt]) => ({ key, savedAt }))
    .sort((a, b) => a.savedAt - b.savedAt)
  const cut = Math.max(0, sorted.length - Math.max(0, max))
  return { kept: sorted.slice(cut), removed: sorted.slice(0, cut).map((e) => e.key) }
}

/** 递归按键名排序的稳定序列化（数组保持原顺序，对象键排序） */
export function canonicalJson(value: unknown): string {
  return JSON.stringify(sortKeys(value))
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys)
  if (value !== null && typeof value === "object") {
    const src = value as Record<string, unknown>
    const out: Record<string, unknown> = {}
    for (const k of Object.keys(src).sort()) out[k] = sortKeys(src[k])
    return out
  }
  return value
}

/** 草稿内容比较（字段顺序无关）；用来判断「草稿是否与服务端版本不同」 */
export function draftsEqual(a: unknown, b: unknown): boolean {
  return canonicalJson(a) === canonicalJson(b)
}

/** 相对时间文案（用于恢复提示；时钟回拨按 0 处理） */
export function formatDraftAge(savedAt: number, now: number = Date.now()): string {
  const minutes = Math.floor(Math.max(0, now - savedAt) / 60_000)
  if (minutes < 1) return "不到 1 分钟前"
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时前`
  return `${Math.floor(hours / 24)} 天前`
}

/** 绝对时间文案（本地时区 YYYY-MM-DD HH:mm，与相对时间互补） */
export function formatDraftClock(savedAt: number): string {
  const d = new Date(savedAt)
  if (Number.isNaN(d.getTime())) return ""
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}
