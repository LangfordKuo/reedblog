import { beforeEach, describe, expect, it } from "vitest"

import { extractTocHeadings, slugifyHeadingText } from "./toc"

describe("slugifyHeadingText", () => {
  it("英文标点折叠为连字符并小写", () => {
    expect(slugifyHeadingText("Hello, World!")).toBe("hello-world")
  })

  it("中文标题（Unicode 字母）原样保留", () => {
    expect(slugifyHeadingText("一杯手冲的时间")).toBe("一杯手冲的时间")
  })

  it("纯符号回退 section", () => {
    expect(slugifyHeadingText("!!! ???")).toBe("section")
  })
})

describe("extractTocHeadings", () => {
  beforeEach(() => {
    document.body.innerHTML = ""
  })

  it("提取 h2/h3（默认不含 h4），无 id 时生成并写回 DOM，重名自动追加后缀", () => {
    document.body.innerHTML = `
      <div id="root">
        <h2>Hello, World!</h2>
        <h2>Hello, World!</h2>
        <h3>细节</h3>
        <h4>太深的标题不提取</h4>
      </div>`
    const root = document.getElementById("root") as HTMLElement
    const toc = extractTocHeadings(root)
    expect(toc).toEqual([
      { id: "hello-world", text: "Hello, World!", level: 2 },
      { id: "hello-world-1", text: "Hello, World!", level: 2 },
      { id: "细节", text: "细节", level: 3 },
    ])
    // id 写回 DOM（锚点跳转依赖）且页面内唯一
    const h2s = root.querySelectorAll("h2")
    expect((h2s[0] as HTMLElement).id).toBe("hello-world")
    expect((h2s[1] as HTMLElement).id).toBe("hello-world-1")
    // 统一写入 scroll-margin-top 偏移
    expect((h2s[0] as HTMLElement).style.scrollMarginTop).toBe("80px")
  })

  it("已有 id 直接沿用，不与生成 id 冲突", () => {
    document.body.innerHTML = `
      <div id="root"><h2 id="custom-anchor">自定义</h2></div>`
    const toc = extractTocHeadings(document.getElementById("root") as HTMLElement)
    expect(toc).toEqual([{ id: "custom-anchor", text: "自定义", level: 2 }])
  })

  it("生成的 id 与页面现存元素 id 冲突时追加 -1 后缀", () => {
    document.body.innerHTML = `
      <div id="root"><h2>Hello World</h2></div>
      <div id="hello-world"></div>`
    const toc = extractTocHeadings(document.getElementById("root") as HTMLElement)
    expect(toc[0].id).toBe("hello-world-1")
  })

  it("剔除 KaTeX 的 .katex-mathml 副本，只取可见排版文本", () => {
    document.body.innerHTML = `
      <div id="root">
        <h2>公式 <span class="katex"><span class="katex-mathml"><math><semantics><annotation>E=mc^2</annotation></semantics></math></span><span class="katex-html" aria-hidden="true">E=mc²</span></span> 标题</h2>
      </div>`
    const toc = extractTocHeadings(document.getElementById("root") as HTMLElement)
    expect(toc).toHaveLength(1)
    expect(toc[0].text).toBe("公式 E=mc² 标题")
    expect(toc[0].text).not.toContain("E=mc^2")
  })

  it("跳过空文本标题", () => {
    document.body.innerHTML = `
      <div id="root"><h2>   </h2><h2>有效标题</h2></div>`
    const toc = extractTocHeadings(document.getElementById("root") as HTMLElement)
    expect(toc.map((h) => h.text)).toEqual(["有效标题"])
  })
})
