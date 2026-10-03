/**
 * 行级差异（契约「文章修订历史」条款）：自研 LCS 实现，不引入新依赖。
 * 用于「选中修订的正文」与「当前编辑中的正文」的对比。
 */

/** same=两侧相同；add=当前编辑内容新增的行；del=修订版本中被删除的行 */
export type DiffLineType = "same" | "add" | "del"

export interface DiffLine {
  type: DiffLineType
  text: string
}

/**
 * LCS 动态规划表规模上限（(n+1)×(m+1) 单元格数）。
 * 超过则退化为「整块删除 + 整块新增」——博客文章量级下几乎不会触发，
 * 但避免超大文本把页面算到卡死（几万行时 O(n×m) 内存不可接受）。
 */
const MAX_DP_CELLS = 1_500_000

/** 文本 → 行数组（保留空行；末尾换行会带来一个空行元素，与编辑器内容一致即可） */
function splitLines(text: string): string[] {
  return text.split("\n")
}

/**
 * oldText（修订版本）→ newText（当前正文）的行级差异。
 * 基于最长公共子序列（LCS）：相同行原样保留，其余按删除/新增输出。
 */
export function diffLines(oldText: string, newText: string): DiffLine[] {
  const a = splitLines(oldText)
  const b = splitLines(newText)
  const n = a.length
  const m = b.length

  // 超限退化为整块替换（不追求最小差异，保证不卡死）
  if ((n + 1) * (m + 1) > MAX_DP_CELLS) {
    return [
      ...a.map((text): DiffLine => ({ type: "del", text })),
      ...b.map((text): DiffLine => ({ type: "add", text })),
    ]
  }

  // dp[i][j] = a[i..] 与 b[j..] 的 LCS 长度；从右下往左上填
  const width = m + 1
  const dp = new Uint32Array((n + 1) * width)
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i * width + j] =
        a[i] === b[j]
          ? dp[(i + 1) * width + j + 1] + 1
          : Math.max(dp[(i + 1) * width + j], dp[i * width + j + 1])
    }
  }

  // 回溯：优先输出「相同」，否则沿 LCS 更长的方向输出删除或新增
  const out: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ type: "same", text: a[i] })
      i++
      j++
    } else if (dp[(i + 1) * width + j] >= dp[i * width + j + 1]) {
      out.push({ type: "del", text: a[i] })
      i++
    } else {
      out.push({ type: "add", text: b[j] })
      j++
    }
  }
  while (i < n) out.push({ type: "del", text: a[i++] })
  while (j < m) out.push({ type: "add", text: b[j++] })
  return out
}
