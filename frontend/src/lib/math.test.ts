import { describe, expect, it } from "vitest"

import { preprocessMathSource } from "./math"

describe("preprocessMathSource", () => {
  it("货币写法「$5 到 $10」不被当作行内公式", () => {
    const out = preprocessMathSource("价格 $5 到 $10 元")
    expect(out).toBe("价格 &#36;5 到 &#36;10 元")
    expect(out).not.toContain("$5")
    expect(out).not.toContain("$10")
  })

  it("合法的行内公式（开标记紧跟数字但后接公式内容）保持原样", () => {
    const src = "面积 $2\\pi r$ 是公式"
    expect(preprocessMathSource(src)).toBe(src)
  })

  it("落单的 $$ 开标记被转义，防止吞掉后续全文", () => {
    const out = preprocessMathSource("前言\n$$\n后面的正文不能被吞")
    expect(out).toBe("前言\n&#36;&#36;\n后面的正文不能被吞")
    expect(out).toContain("后面的正文不能被吞")
  })

  it("成对的 $$ 块级公式标记保持原样", () => {
    const src = "$$\nx^2 + y^2 = z^2\n$$\n后文"
    expect(preprocessMathSource(src)).toBe(src)
  })

  it("独占一行的 $$…$$ 展开为三行块级公式（displayMode）", () => {
    expect(preprocessMathSource("$$E=mc^2$$")).toBe("$$\nE=mc^2\n$$")
  })

  it("围栏代码块内的 $ 原样保留", () => {
    const src = "```sh\necho $5 到 $10\n```"
    expect(preprocessMathSource(src)).toBe(src)
  })

  it("行内代码内的 $ 原样保留", () => {
    const src = "用 `$HOME` 取变量"
    expect(preprocessMathSource(src)).toBe(src)
  })
})
