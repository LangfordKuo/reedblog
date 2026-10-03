/**
 * 原始 HTML 片段注入管线（契约统一的渲染方式与安全策略）：
 * 插件前端轻注入（/api/frontend/injections）与主题自定义组件
 * （widget kind=custom 的 config.html）共用本模块。
 *
 * 信任模型：不转义（插件作者/站长对自己内容负责，与 WordPress 同信任模型），
 * 因此**仅**用于后台/主题包输入的片段，绝不用于任何匿名访客数据。
 * innerHTML 插入的 <script> 不会执行，需重建为可执行 script 元素
 * （统计脚本等是主要用例）。返回已注入的节点，供卸载/重渲染时清理。
 */
export function injectHtmlFragment(
  container: HTMLElement,
  html: string,
  marker: string,
): ChildNode[] {
  const template = document.createElement("template")
  template.innerHTML = html
  const appended: ChildNode[] = []
  for (const node of Array.from(template.content.childNodes)) {
    if (node instanceof HTMLScriptElement) {
      const script = document.createElement("script")
      for (const attr of Array.from(node.attributes)) script.setAttribute(attr.name, attr.value)
      script.textContent = node.textContent
      script.dataset.reedblogFragment = marker
      container.appendChild(script)
      appended.push(script)
    } else {
      if (node instanceof HTMLElement) node.dataset.reedblogFragment = marker
      container.appendChild(node)
      appended.push(node)
    }
  }
  return appended
}
