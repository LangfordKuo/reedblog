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
