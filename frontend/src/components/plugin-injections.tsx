import { useEffect } from "react"

import { api } from "@/lib/api"
import { injectHtmlFragment } from "@/lib/html-inject"

/**
 * 插件前端轻注入：挂载后（hydration 完成）请求一次 GET /api/frontend/injections，
 * head 片段注入 document.head、body_end 片段注入 body 末尾；片段变更（后台改动）后刷新页面生效。
 * 只挂在公开 SiteLayout 上——/admin 与 /install 不经过该 layout，绝不注入。
 * 组件卸载（如 SPA 跳转到后台）时移除已注入节点。自身不渲染任何内容。
 * 注入管线与主题自定义组件共用（lib/html-inject.ts，契约同款安全策略）。
 */
export function PluginInjections() {
  useEffect(() => {
    let cancelled = false
    const injected: ChildNode[] = []

    api
      .frontendInjections()
      .then((data) => {
        if (cancelled) return
        for (const f of data.head ?? []) {
          injected.push(...injectHtmlFragment(document.head, f.html, `plugin:${f.plugin}`))
        }
        for (const f of data.body_end ?? []) {
          injected.push(...injectHtmlFragment(document.body, f.html, `plugin:${f.plugin}`))
        }
      })
      .catch(() => {
        // 注入接口不可用（未装插件/后端未就绪）时静默跳过，不影响页面
      })

    return () => {
      cancelled = true
      for (const node of injected) node.remove()
    }
  }, [])

  return null
}
