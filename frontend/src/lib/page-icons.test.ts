import { BookIcon, FileTextIcon, InfoIcon, LinkIcon, MessageSquareIcon } from "lucide-react"
import { describe, expect, it } from "vitest"

import { DEFAULT_ICON_BY_KIND, ICON_NAMES, getPageIcon } from "./page-icons"

describe("page-icons", () => {
  it("ICON_NAMES：精选表 30~60 个、无重复、全部符合后端 slug 校验", () => {
    expect(ICON_NAMES.length).toBeGreaterThanOrEqual(30)
    expect(ICON_NAMES.length).toBeLessThanOrEqual(60)
    expect(new Set(ICON_NAMES).size).toBe(ICON_NAMES.length)
    // 后端校验：^[a-z0-9-]{0,40}$（前端表里的名字必须都能存进 icon 字段）
    for (const name of ICON_NAMES) {
      expect(name).toMatch(/^[a-z0-9-]{1,40}$/)
    }
    // 内置页的语义图标在表内（新装注入值必须能渲染出来）
    for (const name of ["info", "message-square", "link"]) {
      expect(ICON_NAMES).toContain(name)
    }
  })

  it("命中精选表时返回对应组件", () => {
    expect(getPageIcon("book", "custom")).toBe(BookIcon)
    expect(getPageIcon("message-square", "custom")).toBe(MessageSquareIcon)
  })

  it("icon 为空串 / null / undefined 时回退 kind 默认", () => {
    expect(getPageIcon("", "message_board")).toBe(MessageSquareIcon)
    expect(getPageIcon("", "links")).toBe(LinkIcon)
    expect(getPageIcon(null, "custom")).toBe(FileTextIcon)
    expect(getPageIcon(undefined, "custom")).toBe(FileTextIcon)
  })

  it("未知名字（合法但不在表内）忽略并回退 kind 默认", () => {
    expect(getPageIcon("definitely-not-a-real-icon", "links")).toBe(LinkIcon)
    expect(getPageIcon("not-in-table", "message_board")).toBe(MessageSquareIcon)
  })

  it("custom 且 slug=about 回退 info；其余 custom 回退 file-text", () => {
    expect(getPageIcon("", "custom", "about")).toBe(InfoIcon)
    expect(getPageIcon("unknown-name", "custom", "about")).toBe(InfoIcon)
    expect(getPageIcon("", "custom", "contact")).toBe(FileTextIcon)
    // 自定义图标优先于 about 默认
    expect(getPageIcon("book", "custom", "about")).toBe(BookIcon)
  })

  it("DEFAULT_ICON_BY_KIND 覆盖全部 kind", () => {
    expect(DEFAULT_ICON_BY_KIND.message_board).toBe(MessageSquareIcon)
    expect(DEFAULT_ICON_BY_KIND.links).toBe(LinkIcon)
    expect(DEFAULT_ICON_BY_KIND.custom).toBe(FileTextIcon)
  })
})
