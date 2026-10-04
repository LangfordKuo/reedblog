import type { AuthResult } from "./types"

const TOKEN_KEY = "reedblog_token"
const USER_KEY = "reedblog_username"
const EXPIRES_KEY = "reedblog_token_expires_at"

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY)
}

export function setSession(result: AuthResult): void {
  localStorage.setItem(TOKEN_KEY, result.token)
  localStorage.setItem(USER_KEY, result.username)
  localStorage.setItem(EXPIRES_KEY, result.expires_at)
}

export function clearToken(): void {
  localStorage.removeItem(TOKEN_KEY)
  localStorage.removeItem(USER_KEY)
  localStorage.removeItem(EXPIRES_KEY)
}

export function getStoredUsername(): string | null {
  return localStorage.getItem(USER_KEY)
}

/** 用户名变更通知事件（同页订阅者，如后台侧栏；storage 事件只在其它标签页触发） */
export const USERNAME_CHANGED_EVENT = "reedblog:username-changed"

/** 更新用户名缓存（「用户设置」改名成功后调用，后台侧栏等处立即跟随；不触碰 token） */
export function setStoredUsername(username: string): void {
  localStorage.setItem(USER_KEY, username)
  window.dispatchEvent(new Event(USERNAME_CHANGED_EVENT))
}
