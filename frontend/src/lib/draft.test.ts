import { describe, expect, it } from "vitest"

import {
  DRAFT_INDEX_KEY,
  DRAFT_MAX_CHARS,
  DRAFT_MAX_ENTRIES,
  clearDraft,
  clearPageDraft,
  clearPostDraft,
  draftsEqual,
  draftKey,
  formatDraftAge,
  formatDraftClock,
  pageDraftKey,
  postDraftKey,
  pruneDraftIndex,
  readDraft,
  writeDraft,
  type DraftStorage,
} from "./draft"

/** 内存 Storage 替身（Map 支撑，可断言残留 key） */
function memoryStorage(): DraftStorage & { map: Map<string, string> } {
  const map = new Map<string, string>()
  return {
    map,
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => {
      map.set(k, v)
    },
    removeItem: (k) => {
      map.delete(k)
    },
  }
}

/** 每次写入都抛错的 Storage（模拟隐私模式 / 配额满） */
function throwingStorage(): DraftStorage & { writes: number } {
  const s = {
    writes: 0,
    getItem: () => null,
    setItem: () => {
      s.writes += 1
      throw new DOMException("QuotaExceededError")
    },
    removeItem: () => {},
  }
  return s
}

describe("draftKey：按类型与 id 区分", () => {
  it("文章与页面 key 前缀不同，新建用 new 独立 key", () => {
    expect(postDraftKey(12)).toBe("reedblog:draft:post:12")
    expect(pageDraftKey(7)).toBe("reedblog:draft:page:7")
    expect(postDraftKey("new")).toBe("reedblog:draft:post:new")
    expect(draftKey("page", "new")).toBe("reedblog:draft:page:new")
  })
})

describe("readDraft / writeDraft：信封读写与静默降级", () => {
  it("写入后可读回，savedAt 与 data 完整", () => {
    const s = memoryStorage()
    const ok = writeDraft("k", { title: "标题", n: 1 }, 1000, s)
    expect(ok).toBe(true)
    expect(readDraft<{ title: string; n: number }>("k", s)).toEqual({
      v: 1,
      savedAt: 1000,
      data: { title: "标题", n: 1 },
    })
  })

  it("不存在的 key 与损坏的 JSON 都返回 null，不抛错", () => {
    const s = memoryStorage()
    expect(readDraft("missing", s)).toBeNull()
    s.setItem("broken", "{not json")
    expect(readDraft("broken", s)).toBeNull()
    s.setItem("shape", JSON.stringify({ v: 1, savedAt: "昨天", data: {} }))
    expect(readDraft("shape", s)).toBeNull()
    s.setItem("version", JSON.stringify({ v: 99, savedAt: 1, data: {} }))
    expect(readDraft("version", s)).toBeNull()
  })

  it("storage 不可用（null）时读写都不抛错：读 null、写 false", () => {
    expect(readDraft("k", null)).toBeNull()
    expect(writeDraft("k", { a: 1 }, 1, null)).toBe(false)
    expect(() => clearDraft("k", null)).not.toThrow()
  })

  it("写入抛错（配额满 / 隐私模式）时静默返回 false", () => {
    const s = throwingStorage()
    expect(writeDraft("k", { a: 1 }, 1, s)).toBe(false)
    expect(s.writes).toBe(1)
  })

  it("单条超过字符上限时不存（静默返回 false）", () => {
    const s = memoryStorage()
    const huge = "x".repeat(DRAFT_MAX_CHARS + 1)
    expect(writeDraft("big", huge, 1, s)).toBe(false)
    expect(s.map.size).toBe(0)
  })

  it("clearDraft 删除草稿并从索引移除；clearPostDraft / clearPageDraft 按 id 清理", () => {
    const s = memoryStorage()
    writeDraft(postDraftKey(3), { a: 1 }, 1, s)
    writeDraft(pageDraftKey(3), { a: 2 }, 2, s)
    clearPostDraft(3, s)
    expect(readDraft(postDraftKey(3), s)).toBeNull()
    expect(readDraft(pageDraftKey(3), s)).not.toBeNull()
    clearPageDraft(3, s)
    expect(readDraft(pageDraftKey(3), s)).toBeNull()
    expect(JSON.parse(s.map.get(DRAFT_INDEX_KEY) ?? "[]")).toEqual([])
  })
})

describe("超量清理：索引只保留最近 N 条", () => {
  it("写入超过上限时从最旧的开始清理存储与索引", () => {
    const s = memoryStorage()
    for (let i = 0; i < DRAFT_MAX_ENTRIES + 5; i += 1) {
      writeDraft(`k${i}`, { i }, 1000 + i, s)
    }
    // 前 5 条被清理，后 20 条保留
    for (let i = 0; i < 5; i += 1) expect(readDraft(`k${i}`, s)).toBeNull()
    for (let i = 5; i < DRAFT_MAX_ENTRIES + 5; i += 1) {
      expect(readDraft<{ i: number }>(`k${i}`, s)?.data.i).toBe(i)
    }
    const index = JSON.parse(s.map.get(DRAFT_INDEX_KEY) ?? "[]") as unknown[]
    expect(index).toHaveLength(DRAFT_MAX_ENTRIES)
  })

  it("pruneDraftIndex：同 key 去重保留最新，返回按时间升序的待删列表", () => {
    const { kept, removed } = pruneDraftIndex(
      [
        { key: "a", savedAt: 1 },
        { key: "b", savedAt: 2 },
        { key: "a", savedAt: 3 },
        { key: "c", savedAt: 4 },
      ],
      2,
    )
    expect(removed).toEqual(["b"])
    expect(kept).toEqual([
      { key: "a", savedAt: 3 },
      { key: "c", savedAt: 4 },
    ])
  })
})

describe("draftsEqual：字段顺序无关的内容比较", () => {
  it("对象键顺序不同视为相等；数组顺序不同不相等", () => {
    expect(draftsEqual({ a: 1, b: [2, 3] }, { b: [2, 3], a: 1 })).toBe(true)
    expect(draftsEqual({ tags: [1, 2] }, { tags: [2, 1] })).toBe(false)
    expect(draftsEqual({ a: { x: 1, y: 2 } }, { a: { y: 2, x: 1 } })).toBe(true)
    expect(draftsEqual({ a: 1 }, { a: 1, b: undefined })).toBe(true)
  })
})

describe("时间文案", () => {
  it("formatDraftAge：分钟 / 小时 / 天与未来时钟回拨", () => {
    const now = new Date(2026, 9, 4, 12, 0, 0).getTime()
    expect(formatDraftAge(now - 30_000, now)).toBe("不到 1 分钟前")
    expect(formatDraftAge(now - 5 * 60_000, now)).toBe("5 分钟前")
    expect(formatDraftAge(now - 3 * 3600_000, now)).toBe("3 小时前")
    expect(formatDraftAge(now - 2 * 24 * 3600_000, now)).toBe("2 天前")
    expect(formatDraftAge(now + 60_000, now)).toBe("不到 1 分钟前")
  })

  it("formatDraftClock：本地时区 YYYY-MM-DD HH:mm", () => {
    expect(formatDraftClock(new Date(2026, 9, 4, 8, 5).getTime())).toBe("2026-10-04 08:05")
    expect(formatDraftClock(Number.NaN)).toBe("")
  })
})
