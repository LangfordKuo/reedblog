import { Suspense, useEffect } from "react"
import { Outlet } from "react-router-dom"

import { BlogLeftNav } from "@/components/blog-left-nav"
import { MinimalHeader } from "@/components/minimal-header"
import { PluginInjections } from "@/components/plugin-injections"
import { PostToc } from "@/components/post-toc"
import { SiteFooter } from "@/components/site-footer"
import { BlockSpinner } from "@/components/spinner"
import { SiteHeader } from "@/components/site-header"
import { WidgetRegion } from "@/components/widgets"
import { useSite } from "@/hooks/use-site"
import { useThemeSettings } from "@/lib/theme-settings"
import { shouldRenderToc, useTocHeadings } from "@/lib/toc"
import type { SiteSettings } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useRegionWidgets } from "@/lib/widgets"

/**
 * 公开站点布局壳：按激活主题的 layout 设置分派两套骨架
 * （扩展契约「主题设置项-内置布局设置」，data-layout 同步写在 <html> 上）：
 * - topbar-two-column（默认）：顶栏导航 + 主内容/侧栏双列（侧栏由各页面自带的
 *   WidgetRegion("sidebar") 渲染）；页脚区域由布局壳统一渲染；
 * - topbar-minimal-three-column：极简顶栏（搜索/管理入口），正文左中右三列
 *   （左：导航 + left 区域组件，中：页面内容，右：right/sidebar 降级组件）。
 * 各区域无启用组件时不渲染空壳（WidgetRegion 返回 null；三列右栏空时收缩为两列）。
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
        {/* 路由懒加载：站内跳转只替换内容区，顶栏/页脚外壳保留 */}
        <Suspense fallback={<BlockSpinner />}>
          <Outlet />
        </Suspense>
      </main>
      <WidgetRegion
        region="footer"
        className={cn("mx-auto w-full px-4 pb-8", wide ? "max-w-7xl" : "max-w-5xl")}
      />
      <SiteFooter site={site} wide={wide} />
    </div>
  )
}

/** 极简顶栏 + 左中右三列骨架（左栏含导航与 left 组件，右栏为 TOC + sidebar/right 降级组件） */
function ThreeColumnShell({ site }: { site: SiteSettings | null }) {
  const hasRightWidgets = useRegionWidgets("right").length > 0
  // 文章 TOC（详情页固有部件，不走 widgets 配置）：渲染在右栏最上、组件之前；
  // 详情页写入 store、离开清空，其余页面 hasToc 恒 false 不影响右栏
  const hasToc = shouldRenderToc(useTocHeadings())
  const hasRight = hasRightWidgets || hasToc
  return (
    <div className="flex min-h-screen flex-col bg-background">
      <PluginInjections />
      <MinimalHeader site={site} />
      <main className="mx-auto w-full max-w-7xl flex-1 px-4 py-8">
        <div
          className={cn(
            "grid gap-8",
            hasRight
              ? "lg:grid-cols-[188px_minmax(0,1fr)_288px]"
              : "lg:grid-cols-[188px_minmax(0,1fr)]",
          )}
        >
          <aside className="flex flex-col gap-6 lg:sticky lg:top-20 lg:self-start">
            <BlogLeftNav />
            <WidgetRegion region="left" />
          </aside>
          <div className="min-w-0">
            <Suspense fallback={<BlockSpinner />}>
              <Outlet />
            </Suspense>
          </div>
          {hasRight && (
            <aside
              className={cn(
                "flex flex-col gap-6 lg:sticky lg:top-20 lg:self-start",
                // 仅 TOC（无右栏组件）时窄屏不出空壳（PostToc 自身 hidden lg:block）
                !hasRightWidgets && "hidden lg:flex",
              )}
            >
              <PostToc />
              <WidgetRegion region="right" />
            </aside>
          )}
        </div>
      </main>
      <WidgetRegion region="footer" className="mx-auto w-full max-w-7xl px-4 pb-8" />
      <SiteFooter site={site} wide />
    </div>
  )
}
