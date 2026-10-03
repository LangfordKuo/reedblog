import { useEffect, useState } from "react"

import { loadSiteSettings } from "@/lib/site"
import type { SiteSettings } from "@/lib/types"

/** 加载站点设置（模块级缓存），并同步 document.title */
export function useSite(): { site: SiteSettings | null } {
  const [site, setSite] = useState<SiteSettings | null>(null)

  useEffect(() => {
    let cancelled = false
    loadSiteSettings()
      .then((s) => {
        if (cancelled) return
        setSite(s)
        if (s.title) document.title = s.title
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
