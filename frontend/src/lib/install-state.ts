import { api } from "./api"

// 安装状态模块级缓存：全站只请求一次，安装成功后手动标记
let cached: boolean | null = null
let inflight: Promise<boolean> | null = null

export async function checkInstalled(force = false): Promise<boolean> {
  if (cached !== null && !force) return cached
  if (inflight && !force) return inflight
  inflight = api
    .installStatus()
    .then((r) => {
      cached = r.installed
      return cached
    })
    .finally(() => {
      inflight = null
    })
  return inflight
}

export function markInstalled(): void {
  cached = true
}

/** 同步读取缓存（未加载过则返回 null），用于路由守卫避免闪烁 */
export function peekInstalled(): boolean | null {
  return cached
}
