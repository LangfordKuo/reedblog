import { useEffect, useState } from "react"

import { loadSite } from "@/lib/site"
import type { SiteInfo } from "@/lib/types"

/** 加载站点信息（模块级缓存），并同步 document.title */
export function useSite(): { site: SiteInfo | null } {
  const [site, setSite] = useState<SiteInfo | null>(null)

  useEffect(() => {
    let cancelled = false
    loadSite()
      .then((s) => {
        if (cancelled) return
        setSite(s)
        if (s.title) document.title = s.title
      })
      .catch(() => {
        // 站点信息加载失败不阻塞页面，header 显示默认名
      })
    return () => {
      cancelled = true
    }
  }, [])

  return { site }
}
