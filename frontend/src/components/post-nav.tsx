import { Link } from "react-router-dom"
import { ArrowLeftIcon, ArrowRightIcon } from "lucide-react"

import { cn } from "@/lib/utils"
import type { PostNavItem } from "@/lib/types"

interface PostNavProps {
  /** 上一篇：发布时间更早的相邻文章（契约「文章上一篇/下一篇」条款）；无则 null */
  prev: PostNavItem | null
  /** 下一篇：发布时间更晚的相邻文章；无则 null */
  next: PostNavItem | null
}

/** 文章详情底部「上一篇 / 下一篇」双栏导航（契约「文章上一篇/下一篇」条款）：
 *  左 = 更早、右 = 更晚；单侧为 null 时该侧留空（不出现死链接），窄屏（<sm）上下堆叠。
 *  两侧均无相邻文章时整体不渲染。 */
export function PostNav({ prev, next }: PostNavProps) {
  if (!prev && !next) return null
  return (
    <nav aria-label="文章导航" className="grid gap-3 sm:grid-cols-2">
      {/* 占位元素保证单侧为 null 时另一侧不越列（窄屏隐藏，避免出现空档） */}
      {prev ? <NavLink item={prev} direction="prev" /> : <div className="hidden sm:block" />}
      {next ? <NavLink item={next} direction="next" /> : <div className="hidden sm:block" />}
    </nav>
  )
}

function NavLink({ item, direction }: { item: PostNavItem; direction: "prev" | "next" }) {
  const isPrev = direction === "prev"
  const Icon = isPrev ? ArrowLeftIcon : ArrowRightIcon
  return (
    <Link
      to={`/posts/${encodeURIComponent(item.slug)}`}
      className={cn(
        "flex min-w-0 flex-col gap-1.5 rounded-lg border bg-card p-4 transition-colors",
        "hover:border-foreground/25 hover:bg-accent/50",
        !isPrev && "sm:items-end sm:text-right",
      )}
    >
      <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
        {isPrev && <Icon className="size-3.5" />}
        {isPrev ? "上一篇" : "下一篇"}
        {!isPrev && <Icon className="size-3.5" />}
      </span>
      <span className="line-clamp-2 text-sm font-medium break-words">{item.title}</span>
    </Link>
  )
}
