/**
 * KaTeX 数学公式（`$…$` 行内 / `$$…$$` 块级）的源码预处理与渲染约定。
 *
 * 渲染链路：Markdown 组件（`@/components/markdown`）把 content_md 交给
 * react-markdown 前先跑 `preprocessMathSource`，再由 remark-math 解析出公式节点、
 * rehype-katex（KaTeX）渲染成数学排版。文章详情页正文与编辑器实时预览共用同一个
 * Markdown 组件，因此公式渲染只有这一份实现，两处不会漂移。
 *
 * 预处理纠正 remark-math 实测存在的三处行为（均用 `&#36;` 字符引用转义：
 * 显示结果仍是 `$`，但不会被公式标记识别；实测 `\$` 挡不住闭标记——反斜杠会
 * 被吃进公式内容）：
 * 1. 货币误判：`价格 $5 到 $10 元` 中第二个 `$` 会被当作行内公式的闭标记，
 *    把 `5 到 ` 渲染成公式。规则：闭标记候选（其前已有未闭合的开标记）后紧跟
 *    半角数字时判为货币，转义该 `$`；开标记后紧跟数字是合法写法（如 `$2\pi r$`），
 *    不转义。
 * 2. 未闭合块级公式吞文：`$$` 独占一行但全文再无第二个 `$$` 时，micromark 会把
 *    其后整篇文档都吃进公式节点。规则：按出现顺序两两配对，落单的 `$$` 行转义，
 *    保持原文。
 * 3. 单行 `$$…$$`：remark-math 只在「`$$` 后换行」时产出块级公式，`$$E=mc^2$$`
 *    独占一行会退化成行内公式。规则：把独占一行的 `$$…$$` 展开成三行 math flow，
 *    渲染为块级（displayMode 居中）。
 *
 * 代码区（围栏代码块、行内代码）内的 `$` 原样保留：代码里的美元符号不是公式。
 */

/** 围栏代码块开/闭标记（``` 或 ~~~，≥3 个） */
const FENCE_OPEN = /^(`{3,}|~{3,})/

/** 独占一行的单行块级公式：`$$…$$`（中间不含 `$`） */
const OWN_LINE_MATH = /^\$\$([^$]+)\$\$$/

/** 块级公式闭合行：trim 后仅 `$$` 与可选空白 */
const MATH_FENCE_CLOSE = /^\$\$[ \t]*$/

/** 半角数字（货币防误判只针对半角；全角数字不参与） */
const ASCII_DIGIT = /[0-9]/

/** 显示为 `$` 的字符引用：绕过公式标记识别，渲染输出与原文一致 */
const DOLLAR_ENTITY = "&#36;"

/** 该下标的字符是否被反斜杠转义（数前面连续反斜杠的奇偶） */
function isEscaped(s: string, index: number): boolean {
  let backslashes = 0
  for (let i = index - 1; i >= 0 && s[i] === "\\"; i -= 1) backslashes += 1
  return backslashes % 2 === 1
}

/** 逐行标记围栏代码块区域（含开/闭行本身），这些行整段原样输出 */
function fenceMask(lines: string[]): boolean[] {
  const mask = new Array<boolean>(lines.length).fill(false)
  let fence: string | null = null
  for (let i = 0; i < lines.length; i += 1) {
    const trimmed = lines[i].trimStart()
    if (fence) {
      mask[i] = true
      if (trimmed.startsWith(fence)) fence = null
      continue
    }
    const marker = FENCE_OPEN.exec(trimmed)
    if (marker) {
      mask[i] = true
      fence = marker[1]
    }
  }
  return mask
}

/**
 * `$` 后紧跟半角数字时，判定它是货币写法还是公式开标记。
 * 数字（可含千分位逗号/小数点）之后：
 * - 行尾、空白后的非公式字符（中文、全角标点等）→ 货币（`$5 到 $10`、`$100 美元`）
 * - 公式内容字符（ASCII 字母数字、空白后的运算符/反斜杠命令等）→ 公式
 *   （`$2\pi r$`、`$2 + 2 = 4$`、`$5$`）
 */
function isCurrencyDollar(line: string, index: number): boolean {
  let j = index + 1
  while (j < line.length && /[0-9,.]/.test(line[j])) j += 1
  while (j < line.length && /\s/.test(line[j])) j += 1
  if (j >= line.length) return true
  return !/[0-9A-Za-z\\^_=+\-*/()[\]{}|<>%!$]/.test(line[j])
}

/**
 * 行内货币防误判：把判为货币的 `$` 换成显示等价的字符引用（不参与公式标记识别）。
 * 代码区（行内代码反引号段）整段跳过；`$$` 序列交给块级逻辑处理。
 */
function escapeCurrencyDollars(line: string): string {
  let out = ""
  let i = 0
  while (i < line.length) {
    const ch = line[i]
    // 行内代码 `…`：整段原样输出（代码里的 $ 不是公式）；未闭合反引号按字面字符继续
    if (ch === "`") {
      let run = 0
      while (line[i + run] === "`") run += 1
      const marker = "`".repeat(run)
      const close = line.indexOf(marker, i + run)
      if (close !== -1) {
        out += line.slice(i, close + run)
        i = close + run
        continue
      }
    }
    if (ch === "$" && !isEscaped(line, i)) {
      const next = line[i + 1]
      const isDouble = next === "$" || line[i - 1] === "$"
      if (!isDouble && next !== undefined && ASCII_DIGIT.test(next) && isCurrencyDollar(line, i)) {
        out += DOLLAR_ENTITY
        i += 1
        continue
      }
    }
    out += ch
    i += 1
  }
  return out
}

/**
 * 潜在块级公式开标记行：纯 `$$` 行，或 `$$` 后有内容且该行无第二个 `$$`
 * （缩进 <4 时才算，避免误伤缩进代码块）。单行 `$$…$$` 另行展开，不参与配对。
 */
function isPotentialFenceOpen(trimmed: string): boolean {
  if (!trimmed.startsWith("$$")) return false
  if (MATH_FENCE_CLOSE.test(trimmed)) return true
  if (OWN_LINE_MATH.test(trimmed)) return false
  return trimmed.indexOf("$$", 2) === -1
}

/**
 * Markdown 源码的公式预处理（详情页正文与编辑器预览共用）：
 * 围栏代码块整段原样；落单的 `$$` 开标记转义（防吞掉后续全文）；独占一行的
 * `$$…$$` 展开为块级；其余行做货币防误判转义。输出只可能新增 `&#36;` 实体
 * 与块级公式换行，渲染结果与作者书写一致。
 */
export function preprocessMathSource(src: string): string {
  const lines = src.split("\n")
  const inFence = fenceMask(lines)

  // 未闭合块级公式：围栏外、缩进 <4 的开标记行按出现顺序两两配对，落单的最后一个转义
  const opens: number[] = []
  for (let i = 0; i < lines.length; i += 1) {
    const trimmed = lines[i].trimStart()
    const indent = lines[i].length - trimmed.length
    if (!inFence[i] && indent < 4 && isPotentialFenceOpen(trimmed)) opens.push(i)
  }
  const orphan = opens.length % 2 === 1 ? opens[opens.length - 1] : -1

  const out: string[] = []
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i]
    if (inFence[i]) {
      out.push(line)
      continue
    }
    const trimmed = line.trimStart()
    const indent = line.slice(0, line.length - trimmed.length)
    if (i === orphan) {
      out.push(`${indent}${DOLLAR_ENTITY}${DOLLAR_ENTITY}${trimmed.slice(2)}`)
      continue
    }
    // 独占一行的 `$$…$$`（缩进 <4，避免误伤缩进代码块）→ 展开为块级 math flow
    const own = indent.length < 4 ? OWN_LINE_MATH.exec(trimmed) : null
    if (own) {
      out.push(`${indent}$$`, `${indent}${own[1].trim()}`, `${indent}$$`)
      continue
    }
    out.push(escapeCurrencyDollars(line))
  }
  return out.join("\n")
}
