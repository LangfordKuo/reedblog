import { api } from "./api"
import type { SiteSettings } from "./types"

// 站点设置模块级缓存（标题/副标题/页脚/per_page 全站共用，避免每页重复请求）
let inflight: Promise<SiteSettings> | null = null

/** 加载公开站点设置；force=true 时绕过缓存重新拉取（后台保存设置后刷新缓存用） */
export function loadSiteSettings(force = false): Promise<SiteSettings> {
  if (!inflight || force) {
    inflight = api.siteSettings().catch((e) => {
      inflight = null
      throw e
    })
  }
  return inflight
}
