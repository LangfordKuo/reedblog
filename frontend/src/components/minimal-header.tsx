import { useEffect, useState, type FormEvent } from "react"
import { Link, useNavigate } from "react-router-dom"
import { MoonIcon, SearchIcon, SunIcon } from "lucide-react"

import { Button, buttonVariants } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { isDarkRendered, setColorMode } from "@/lib/color-mode"
import type { SiteSettings } from "@/lib/types"
import { cn } from "@/lib/utils"

/**
 * 极简顶栏（layout=topbar-minimal-three-column 骨架专用）：
 * 不含导航链接（导航移到左栏），仅保留站点名、搜索、暗色切换与后台管理入口。
 */
export function MinimalHeader({ site }: { site: SiteSettings | null }) {
  const navigate = useNavigate()
  const [searchOpen, setSearchOpen] = useState(false)
  const [keyword, setKeyword] = useState("")
  const [dark, setDark] = useState(isDarkRendered)

  // system 态下跟随系统偏好变化刷新图标（与 SiteHeader 同款机制）
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)")
    const sync = () => setDark(isDarkRendered())
    mq.addEventListener("change", sync)
    return () => mq.removeEventListener("change", sync)
  }, [])

  const submitSearch = (e: FormEvent) => {
    e.preventDefault()
    const kw = keyword.trim()
    setSearchOpen(false)
    navigate(kw ? `/search?q=${encodeURIComponent(kw)}` : "/search")
  }

  const toggleDark = () => {
    setColorMode(dark ? "light" : "dark")
    setDark(isDarkRendered())
  }

  return (
    <header className="sticky top-0 z-40 border-b bg-background/90 backdrop-blur">
      <div className="mx-auto flex h-14 w-full max-w-7xl items-center justify-between gap-4 px-4">
        <Link to="/" className="flex min-w-0 items-baseline gap-2">
          <span className="truncate text-base font-bold tracking-tight">
            {site?.title || "reedblog"}
          </span>
          {site?.subtitle && (
            <span className="hidden truncate text-xs text-muted-foreground sm:block">
              {site.subtitle}
            </span>
          )}
        </Link>
        <div className="flex items-center gap-1">
          {searchOpen ? (
            <form onSubmit={submitSearch} className="mr-1 flex items-center">
              <Input
                autoFocus
                value={keyword}
                onChange={(e) => setKeyword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    setSearchOpen(false)
                    setKeyword("")
                  }
                }}
                placeholder="搜索…"
                aria-label="搜索关键词"
                className="h-8 w-32 sm:w-48"
              />
            </form>
          ) : (
            <Button
              variant="ghost"
              size="icon"
              className="size-8 text-muted-foreground"
              aria-label="打开搜索"
              onClick={() => setSearchOpen(true)}
            >
              <SearchIcon className="size-4" />
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon"
            className="size-8 text-muted-foreground"
            aria-label={dark ? "切换为亮色模式" : "切换为暗色模式"}
            title={dark ? "切换为亮色模式" : "切换为暗色模式"}
            onClick={toggleDark}
          >
            {dark ? <SunIcon className="size-4" /> : <MoonIcon className="size-4" />}
          </Button>
          <Link
            to="/admin"
            className={cn(buttonVariants({ variant: "ghost", size: "sm" }), "ml-1")}
          >
            管理
          </Link>
        </div>
      </div>
    </header>
  )
}
