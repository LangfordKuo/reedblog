import { describe, expect, it } from "vitest"

import { formatBytes, formatDate, formatDateTime } from "./utils"

describe("formatBytes", () => {
  it("0 与 1KB 以下按字节显示", () => {
    expect(formatBytes(0)).toBe("0 B")
    expect(formatBytes(1)).toBe("1 B")
    expect(formatBytes(1023)).toBe("1023 B")
  })

  it("1KB 及以上按 KB（保留一位小数）", () => {
    expect(formatBytes(1024)).toBe("1.0 KB")
    expect(formatBytes(1536)).toBe("1.5 KB")
    expect(formatBytes(1024 * 1024 - 1)).toBe("1024.0 KB")
  })

  it("1MB 及以上按 MB", () => {
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB")
    expect(formatBytes(5 * 1024 * 1024 + 512 * 1024)).toBe("5.5 MB")
  })

  it("负数 / NaN / 无穷大返回占位符", () => {
    expect(formatBytes(-1)).toBe("-")
    expect(formatBytes(Number.NaN)).toBe("-")
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe("-")
  })
})

describe("formatDate / formatDateTime", () => {
  it("空值返回占位符", () => {
    expect(formatDate(null)).toBe("-")
    expect(formatDate(undefined)).toBe("-")
    expect(formatDateTime(null)).toBe("-")
  })

  it("非法时间字符串原样返回", () => {
    expect(formatDate("not-a-date")).toBe("not-a-date")
    expect(formatDateTime("not-a-date")).toBe("not-a-date")
  })

  it("RFC3339 时间戳转本地日期与日期时间", () => {
    // 不带时区后缀的 ISO 字符串按本地时间解析，跨时区结果稳定
    expect(formatDate("2024-03-05T12:30:00")).toBe("2024-03-05")
    expect(formatDateTime("2024-03-05T12:30:00")).toBe("2024-03-05 12:30")
    expect(formatDateTime("2024-03-05T07:05:00")).toBe("2024-03-05 07:05")
  })
})
