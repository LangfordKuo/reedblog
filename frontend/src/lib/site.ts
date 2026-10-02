import { api } from "./api"
import type { SiteInfo } from "./types"

// 站点信息模块级缓存（标题/副标题全站共用）
let inflight: Promise<SiteInfo> | null = null

export function loadSite(force = false): Promise<SiteInfo> {
  if (!inflight || force) {
    inflight = api.site().catch((e) => {
      inflight = null
      throw e
    })
  }
  return inflight
}
