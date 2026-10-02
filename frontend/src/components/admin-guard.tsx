import { useEffect, useState } from "react"
import { Navigate, Outlet, useLocation } from "react-router-dom"

import { FullPageSpinner } from "@/components/spinner"
import { api } from "@/lib/api"
import { clearToken, getToken } from "@/lib/auth"

/**
 * 管理后台守卫：无 token 或 GET /api/auth/me 校验失败时跳转登录页，
 * 并通过 state.from 记录来源以便登录后回跳。
 */
export function AdminGuard() {
  const location = useLocation()
  const [status, setStatus] = useState<"loading" | "ok" | "fail">(() =>
    getToken() ? "loading" : "fail",
  )

  useEffect(() => {
    if (!getToken()) {
      setStatus("fail")
      return
    }
    let cancelled = false
    api
      .me()
      .then(() => {
        if (!cancelled) setStatus("ok")
      })
      .catch(() => {
        if (!cancelled) {
          clearToken()
          setStatus("fail")
        }
      })
    return () => {
      cancelled = true
    }
  }, [location.pathname])

  if (status === "loading") return <FullPageSpinner label="正在验证登录状态…" />
  if (status === "fail") return <Navigate to="/admin/login" replace state={{ from: location }} />
  return <Outlet />
}
