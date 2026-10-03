// 点赞匿名 id（契约「浏览量与点赞」条款）：前端首次生成 UUID 存
// localStorage `reedblog_like_id`，点赞三接口都带上；后端按 (post_id, liker_key)
// UNIQUE 去重。匿名 id 定位为轻量互动标识（清存储/换浏览器即视为新访客）。

const STORAGE_KEY = "reedblog_like_id"

/** localStorage 不可用（隐私模式等）时的进程内兜底 id */
let memoryFallback: string | null = null

/** 生成 UUID：优先 crypto.randomUUID，缺失时手动拼 v4 形状 */
function newUuid(): string {
  const c: Crypto | undefined = typeof crypto !== "undefined" ? crypto : undefined
  if (c && typeof c.randomUUID === "function") {
    return c.randomUUID()
  }
  const bytes = new Uint8Array(16)
  if (c && typeof c.getRandomValues === "function") {
    c.getRandomValues(bytes)
  } else {
    for (let i = 0; i < 16; i++) bytes[i] = Math.floor(Math.random() * 256)
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x40 // version 4
  bytes[8] = (bytes[8] & 0x3f) | 0x80 // variant 10
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("")
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

/** 取当前访客的点赞匿名 id（首次调用生成并持久化） */
export function getLikeId(): string {
  try {
    const existing = localStorage.getItem(STORAGE_KEY)
    if (existing && existing.trim()) return existing.trim()
    const id = newUuid()
    localStorage.setItem(STORAGE_KEY, id)
    return id
  } catch {
    // localStorage 不可用：会话内稳定的内存兜底
    if (!memoryFallback) memoryFallback = newUuid()
    return memoryFallback
  }
}
