import { useEffect, useState, type MouseEvent } from "react"
import { ListIcon } from "lucide-react"

import { WidgetShell } from "@/components/widgets/widget-shell"
import { TOC_ACTIVE_LINE_PX, shouldRenderToc, useTocHeadings } from "@/lib/toc"
import { cn } from "@/lib/utils"

/**
 * 文章目录（TOC）：文章详情页固有部件（不走 widgets 配置）。
 * - 数据来自 lib/toc 全局 store（详情页正文渲染后提取写入）；
 * - 标题数 < TOC_MIN_HEADINGS 时整体不渲染；
 * - 移动端（<lg）不渲染（hidden lg:block）——取舍：窄屏右栏本就堆叠到
 *   正文下方，TOC 跟随滚动才有效、置于页尾无导航价值，隐藏保持 DOM 简洁；
 * - 当前阅读位置高亮：scroll 监听（rAF 节流）取「顶部滚过判定线的最后
 *   一个标题」，滚至页底强制选中末项（末节很短时也能高亮）；
 * - 点击平滑滚动：标题元素带 scroll-margin-top（提取时写入），
 *   scrollIntoView 自动避开 sticky header；同步替换 URL hash（不进历史栈）。
 * 配色全用现有 tokens；高亮色条消费主题 accent_color 变量（未设置回退 --primary），
 * 暗色模式自动适配。
 */

/** 缩进映射：h3 相对 h2 缩进一级，h4（TOC_MAX_LEVEL=4 时）再进一级 */
const INDENT: Record<number, string> = { 2: "pl-3", 3: "pl-6", 4: "pl-9" }

export function PostToc() {
  const headings = useTocHeadings()
  const [activeId, setActiveId] = useState<string | null>(null)
  const visible = shouldRenderToc(headings)

  // 阅读位置跟踪：scroll/resize → rAF 节流重算 activeId
  useEffect(() => {
    if (!visible) return
    let raf = 0
    const update = () => {
      raf = 0
      const doc = document.documentElement
      const atBottom = window.innerHeight + window.scrollY >= doc.scrollHeight - 4
      let next: string | null = null
      if (atBottom) {
        next = headings[headings.length - 1]?.id ?? null
      } else {
        for (const h of headings) {
          const el = document.getElementById(h.id)
          if (!el) continue
          if (el.getBoundingClientRect().top <= TOC_ACTIVE_LINE_PX) next = h.id
          else break
        }
      }
      setActiveId((prev) => (prev === next ? prev : next))
    }
    const onScroll = () => {
      if (!raf) raf = requestAnimationFrame(update)
    }
    update()
    window.addEventListener("scroll", onScroll, { passive: true })
    window.addEventListener("resize", onScroll)
    return () => {
      if (raf) cancelAnimationFrame(raf)
      window.removeEventListener("scroll", onScroll)
      window.removeEventListener("resize", onScroll)
    }
  }, [visible, headings])

  if (!visible) return null

  const onClick = (e: MouseEvent<HTMLAnchorElement>, id: string) => {
    e.preventDefault()
    const el = document.getElementById(id)
    if (!el) return
    el.scrollIntoView({ behavior: "smooth", block: "start" })
    setActiveId(id)
    window.history.replaceState(null, "", `#${id}`)
  }

  return (
    <nav aria-label="文章目录" className="hidden lg:block">
      <WidgetShell title="目录" icon={<ListIcon className="size-4 text-muted-foreground" />}>
        <ul className="flex flex-col gap-0.5">
          {headings.map((h) => {
            const active = h.id === activeId
            return (
              <li key={h.id}>
                <a
                  href={`#${h.id}`}
                  onClick={(e) => onClick(e, h.id)}
                  aria-current={active ? "location" : undefined}
                  style={{
                    borderLeftColor: active
                      ? "var(--theme-setting-accent-color, var(--primary))"
                      : "transparent",
                  }}
                  className={cn(
                    "block border-l-2 py-1 text-sm leading-snug break-words transition-colors",
                    INDENT[h.level] ?? "pl-3",
                    active
                      ? "font-medium text-foreground"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {h.text}
                </a>
              </li>
            )
          })}
        </ul>
      </WidgetShell>
    </nav>
  )
}
