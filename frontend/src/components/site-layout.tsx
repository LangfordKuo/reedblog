import { useEffect } from "react"
import { Outlet } from "react-router-dom"

import { PluginInjections } from "@/components/plugin-injections"
import { SiteFooter } from "@/components/site-footer"
import { SiteHeader } from "@/components/site-header"
import { useSite } from "@/hooks/use-site"

export function SiteLayout() {
  const { site } = useSite()

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

  return (
    <div className="flex min-h-screen flex-col bg-background">
      {/* 插件前端轻注入：仅公开 layout 挂载，/admin 与 /install 绝不注入 */}
      <PluginInjections />
      <SiteHeader site={site} />
      <main className="mx-auto w-full max-w-5xl flex-1 px-4 py-8">
        <Outlet />
      </main>
      <SiteFooter site={site} />
    </div>
  )
}
