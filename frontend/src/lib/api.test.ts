import { afterEach, describe, expect, it, vi } from "vitest"

import { ApiError, api, errorMessage } from "./api"

/** 用给定状态码/响应头替换全局 fetch，并返回可断言的 mock */
function stubFetch(status: number, headers: Record<string, string>, body: unknown) {
  const mock = vi.fn(
    async () =>
      new Response(JSON.stringify(body), {
        status,
        headers: { "Content-Type": "application/json", ...headers },
      }),
  )
  vi.stubGlobal("fetch", mock)
  return mock
}

describe("ApiError.retryAfter（反滥用限流）", () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it("429 带 Retry-After: 60 → retryAfter 解析为 60，错误码与文案保留", async () => {
    const mock = stubFetch(429, { "Retry-After": "60" }, {
      error: { code: "rate_limited", message: "评论过于频繁，请稍后再试" },
    })
    const err = await api
      .createComment("hello", { author_name: "读者", content: "沙发" })
      .catch((e: unknown) => e)

    expect(mock).toHaveBeenCalledTimes(1)
    expect(err).toBeInstanceOf(ApiError)
    expect((err as ApiError).status).toBe(429)
    expect((err as ApiError).code).toBe("rate_limited")
    expect((err as ApiError).retryAfter).toBe(60)
    expect(errorMessage(err)).toBe("评论过于频繁，请稍后再试")
  })

  it("429 缺 Retry-After 头 → retryAfter 为 undefined", async () => {
    stubFetch(429, {}, { error: { code: "rate_limited", message: "稍后再试" } })
    const err = await api.createComment("hello", { author_name: "读者", content: "沙发" }).catch((e: unknown) => e)
    expect((err as ApiError).retryAfter).toBeUndefined()
  })

  it("Retry-After 为 HTTP-date 等非整数秒格式时不解析", async () => {
    stubFetch(429, { "Retry-After": "Wed, 21 Oct 2026 07:28:00 GMT" }, {
      error: { code: "rate_limited", message: "稍后再试" },
    })
    const err = await api.createComment("hello", { author_name: "读者", content: "沙发" }).catch((e: unknown) => e)
    expect((err as ApiError).retryAfter).toBeUndefined()
  })
})
