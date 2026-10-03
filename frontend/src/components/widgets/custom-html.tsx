import { useEffect, useRef } from "react"

import { WidgetShell, cfgStr, type WidgetProps } from "./widget-shell"
import { injectHtmlFragment } from "@/lib/html-inject"

/**
 * 自定义 HTML 组件（kind=custom）：渲染 config.html（公开端点已完成 {{param}} 替换）。
 * 与插件前端轻注入共用同一份渲染管线与安全策略（lib/html-inject.ts）：
 * dangerouslySetInnerHTML 级别的原始注入仅用于此类后台/主题包输入的片段，
 * <script> 重建为可执行元素。html 变更时重新注入，卸载时清理已注入节点。
 */
export function CustomHtmlWidget({ widget }: WidgetProps) {
  const title = cfgStr(widget.config, "title", "")
  const html = typeof widget.config.html === "string" ? widget.config.html : ""
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    // 重新注入前先清空上一版片段（含已执行脚本重建的节点）
    el.replaceChildren()
    const injected = html ? injectHtmlFragment(el, html, `widget:${widget.key}`) : []
    return () => {
      for (const node of injected) node.remove()
    }
  }, [html, widget.key])

  const body = <div ref={containerRef} className="widget-html text-sm" />

  // 有标题时套卡片外壳，无标题时裸容器（片段自带样式）
  return title ? <WidgetShell title={title}>{body}</WidgetShell> : body
}
