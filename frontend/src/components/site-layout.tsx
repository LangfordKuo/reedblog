import { useEffect } from "react"
import { Outlet } from "react-router-dom"

import { BlogLeftNav } from "@/components/blog-left-nav"
import { BlogSidebar } from "@/components/blog-sidebar"
import { MinimalHeader } from "@/components/minimal-header"
import { PluginInjections } from "@/components/plugin-injections"
import { SiteFooter } from "@/components/site-footer"
import { SiteHeader } from "@/components/site-header"
import { useSite } from "@/hooks/use-site"
import { useThemeSettings } from "@/lib/theme-settings"
import type { SiteSettings } from "@/lib/types"
import { cn } from "@/lib/utils"

/**
 * 公开站点布局壳：按激活主题的 layout 设置分派两套骨架
 * （扩展契约「主题设置项-内置布局设置」，data-layout 同步写在 <html> 上）：
 * - topbar-two-column（默认）：顶栏导航 + 主内容/侧栏双列（侧栏由各页面自带）；
 * - topbar-minimal-three-column：极简顶栏（搜索/管理入口），正文左中右三列
 *   （左：导航/分类，中：页面内容，右：侧栏信息）。
 * 其余页面（详情/归档等）复用同一布局壳。
 */
export function SiteLayout() {
  const { site } = useSite()
  const { layout } = useThemeSettings()

  // RSS 自动发现：head 注入 <link rel="alternate">（仅公开 layout；卸载时移除）
  useEffect(() => {
    const link = document.createElement("link")
    link.rel = "alternate"
    link.type = "application/rss+xml"
    link.title = site?.title ? `${site.title} RSS` : "RSS"
    link.href = "/api/feed.xml"
    document.head.appendChild(link)
    return () => {
      link.remove()
    }
  }, [site?.title])

  if (layout === "topbar-minimal-three-column") {
    return <ThreeColumnShell site={site} />
  }
  return <TwoColumnShell site={site} />
}

/** 顶栏导航 + 双列骨架（现状布局；wide_layout 设置放宽容器宽度） */
function TwoColumnShell({ site }: { site: SiteSettings | null }) {
  const { values } = useThemeSettings()
  const wide = values.wide_layout === true
  return (
    <div className="flex min-h-screen flex-col bg-background">
      {/* 插件前端轻注入：仅公开 layout 挂载，/admin 与 /install 绝不注入 */}
      <PluginInjections />
      <SiteHeader site={site} wide={wide} />
      <main
        className={cn(
          "mx-auto w-full flex-1 px-4 py-8",
          wide ? "max-w-7xl" : "max-w-5xl",
        )}
      >
        <Outlet />
      </main>
      <SiteFooter site={site} wide={wide} />
    </div>
  )
}

/** 极简顶栏 + 左中右三列骨架（页面内容自带的侧栏在此布局下隐藏，右栏统一提供） */
function ThreeColumnShell({ site }: { site: SiteSettings | null }) {
  return (
    <div className="flex min-h-screen flex-col bg-background">
      <PluginInjections />
      <MinimalHeader site={site} />
      <main className="mx-auto w-full max-w-7xl flex-1 px-4 py-8">
        <div className="grid gap-8 lg:grid-cols-[188px_minmax(0,1fr)_288px]">
          <aside className="lg:sticky lg:top-20 lg:self-start">
            <BlogLeftNav />
          </aside>
          <div className="min-w-0">
            <Outlet />
          </div>
          <aside className="lg:sticky lg:top-20 lg:self-start">
            <BlogSidebar />
          </aside>
        </div>
      </main>
      <SiteFooter site={site} wide />
    </div>
  )
}
