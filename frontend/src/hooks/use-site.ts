import { useEffect, useState } from "react"

import { loadSiteSettings } from "@/lib/site"
import type { SiteSettings } from "@/lib/types"

/** 加载站点设置（模块级缓存），供布局头部/页脚渲染。
 *  注意：document.title 由 `lib/meta.ts` 的 applyPageMeta 统一管理（避免页面标题
 *  被这里的异步回调覆盖成纯站点名），本 hook 不再触碰 head。 */
export function useSite(): { site: SiteSettings | null } {
  const [site, setSite] = useState<SiteSettings | null>(null)

  useEffect(() => {
    let cancelled = false
    loadSiteSettings()
      .then((s) => {
        if (cancelled) return
        setSite(s)
      })
      .catch(() => {
        // 站点设置加载失败不阻塞页面，header 显示默认名
      })
    return () => {
      cancelled = true
    }
  }, [])

  return { site }
}
