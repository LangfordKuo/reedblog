import { useEffect, useState } from "react"

/**
 * 阅读进度条：页面顶部 2px，随滚动增长（scaleX 变换，仅合成层开销）。
 * 颜色消费主题 accent_color 设置变量（--theme-setting-accent-color），
 * 未设置时回退 --primary token（暗色模式自动适配）。
 * fixed z-50 覆盖在 sticky header（z-40）顶缘；纯装饰，aria-hidden。
 */
export function ReadingProgress() {
  const [ratio, setRatio] = useState(0)

  useEffect(() => {
    let raf = 0
    const update = () => {
      raf = 0
      const doc = document.documentElement
      const max = doc.scrollHeight - window.innerHeight
      setRatio(max > 0 ? Math.min(1, Math.max(0, window.scrollY / max)) : 0)
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
  }, [])

  return (
    <div aria-hidden className="pointer-events-none fixed inset-x-0 top-0 z-50 h-[2px]">
      <div
        className="h-full origin-left"
        style={{
          transform: `scaleX(${ratio})`,
          background: "var(--theme-setting-accent-color, var(--primary))",
        }}
      />
    </div>
  )
}
