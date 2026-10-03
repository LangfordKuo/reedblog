import { describe, expect, it } from "vitest"

import { diffLines } from "./diff"

describe("diffLines", () => {
  it("完全相同 → 全部 same，无任何增删", () => {
    const text = "第一行\n第二行\n第三行"
    const out = diffLines(text, text)
    expect(out.map((l) => l.type)).toEqual(["same", "same", "same"])
    expect(out.map((l) => l.text)).toEqual(["第一行", "第二行", "第三行"])
    expect(out.some((l) => l.type === "add" || l.type === "del")).toBe(false)
  })

  it("单行改写 → 1 删 1 增，上下文保持 same", () => {
    const out = diffLines("a\nb\nc", "a\nB\nc")
    expect(out).toEqual([
      { type: "same", text: "a" },
      { type: "del", text: "b" },
      { type: "add", text: "B" },
      { type: "same", text: "c" },
    ])
    expect(out.filter((l) => l.type === "del")).toHaveLength(1)
    expect(out.filter((l) => l.type === "add")).toHaveLength(1)
  })

  it("中间插入行 → 只有新增，不改动原有行", () => {
    const out = diffLines("a\nc", "a\nb\nc")
    expect(out).toEqual([
      { type: "same", text: "a" },
      { type: "add", text: "b" },
      { type: "same", text: "c" },
    ])
    expect(out.filter((l) => l.type === "del")).toHaveLength(0)
  })

  it("中间删除行 → 只有删除，不改动原有行", () => {
    const out = diffLines("a\nb\nc", "a\nc")
    expect(out).toEqual([
      { type: "same", text: "a" },
      { type: "del", text: "b" },
      { type: "same", text: "c" },
    ])
    expect(out.filter((l) => l.type === "add")).toHaveLength(0)
  })

  it("超过 DP 单元格上限 → 退化为整块删除 + 整块新增", () => {
    // 1226 × 1226 = 1,503,076 > MAX_DP_CELLS(1,500,000)
    const n = 1225
    expect((n + 1) * (n + 1)).toBeGreaterThan(1_500_000)
    const oldText = Array.from({ length: n }, (_, i) => `old-${i}`).join("\n")
    const newText = Array.from({ length: n }, (_, i) => `new-${i}`).join("\n")
    const out = diffLines(oldText, newText)
    expect(out).toHaveLength(2 * n)
    expect(out.slice(0, n).every((l) => l.type === "del")).toBe(true)
    expect(out.slice(n).every((l) => l.type === "add")).toBe(true)
    expect(out.some((l) => l.type === "same")).toBe(false)
  })
})
