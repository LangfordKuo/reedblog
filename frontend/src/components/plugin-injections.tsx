import { useEffect } from "react"

import { api } from "@/lib/api"

/**
 * 把一段原始 HTML 注入指定容器（不转义，契约：插件作者对自己内容负责，
 * 与 WordPress 插件同信任模型）。innerHTML 插入的 <script> 不会执行，
 * 需重建为可执行 script 元素（统计脚本等是 head 注入的主要用例）。
 * 返回已注入的节点，供卸载时清理。
 */
function injectFragment(container: HTMLElement, html: string, slug: string): ChildNode[] {
  const template = document.createElement("template")
  template.innerHTML = html
  const appended: ChildNode[] = []
  for (const node of Array.from(template.content.childNodes)) {
    if (node instanceof HTMLScriptElement) {
      const script = document.createElement("script")
      for (const attr of Array.from(node.attributes)) script.setAttribute(attr.name, attr.value)
      script.textContent = node.textContent
      script.dataset.reedblogPlugin = slug
      container.appendChild(script)
      appended.push(script)
    } else {
      if (node instanceof HTMLElement) node.dataset.reedblogPlugin = slug
      container.appendChild(node)
      appended.push(node)
    }
  }
  return appended
}

/**
 * 插件前端轻注入：挂载后（hydration 完成）请求一次 GET /api/frontend/injections，
 * head 片段注入 document.head、body_end 片段注入 body 末尾；片段变更（后台改动）后刷新页面生效。
 * 只挂在公开 SiteLayout 上——/admin 与 /install 不经过该 layout，绝不注入。
 * 组件卸载（如 SPA 跳转到后台）时移除已注入节点。自身不渲染任何内容。
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
          injected.push(...injectFragment(document.head, f.html, f.plugin))
        }
        for (const f of data.body_end ?? []) {
          injected.push(...injectFragment(document.body, f.html, f.plugin))
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
