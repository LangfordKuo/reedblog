import { Fragment } from "react"

import { cn } from "@/lib/utils"

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

interface HighlightProps {
  text: string
  /** 搜索词条（与后端一致：空白切分后的词条，大小写不敏感命中） */
  terms: string[]
  className?: string
}

/**
 * 把文本中命中的搜索词条包成 <mark>（大小写不敏感）。
 * 纯 React 节点实现：split（带捕获组）+ map，绝不使用 dangerouslySetInnerHTML。
 */
export function Highlight({ text, terms, className }: HighlightProps) {
  // 去重去空、长词优先（正则交替按序匹配，长词在前避免被短词截胡）
  const uniq = [...new Set(terms.map((t) => t.trim()).filter(Boolean))].sort(
    (a, b) => b.length - a.length,
  )
  if (uniq.length === 0 || !text) return <>{text}</>

  const re = new RegExp(`(${uniq.map(escapeRegExp).join("|")})`, "gi")
  // 带捕获组的 split：奇数位即命中片段
  const parts = text.split(re)
  return (
    <>
      {parts.map((part, i) =>
        i % 2 === 1 ? (
          <mark
            key={i}
            className={cn(
              "rounded-[2px] bg-amber-200/70 px-0.5 text-inherit dark:bg-amber-400/30",
              className,
            )}
          >
            {part}
          </mark>
        ) : (
          <Fragment key={i}>{part}</Fragment>
        ),
      )}
    </>
  )
}
