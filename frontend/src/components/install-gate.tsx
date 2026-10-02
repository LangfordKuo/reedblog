import { useEffect, useState } from "react"
import { Navigate, Outlet, useLocation } from "react-router-dom"

import { BackendDownScreen, FullPageSpinner } from "@/components/spinner"
import { checkInstalled, peekInstalled } from "@/lib/install-state"

type GateState =
  | { status: "loading" }
  | { status: "error" }
  | { status: "ready"; installed: boolean }

/**
 * 全站安装守卫：
 * - 未安装时，除 /install 外全部重定向到 /install
 * - 已安装时，访问 /install 重定向到首页
 * - 后端不可用时展示重试页面
 */
export function InstallGate() {
  const { pathname } = useLocation()
  const [state, setState] = useState<GateState>(() => {
    const cached = peekInstalled()
    return cached === null ? { status: "loading" } : { status: "ready", installed: cached }
  })

  // 渲染期同步模块级缓存：安装页提交成功后 markInstalled() 只更新了模块缓存，
  // 本地 state 仍是旧的 installed:false，会把已跳转的用户弹回 /install。
  // 这里在渲染阶段（pathname 变化触发的重渲染）比对缓存与 state，不一致才 setState。
  // 必须放在渲染期而非 useEffect：effect 在提交后才执行，旧 state 渲染出的
  // <Navigate to="/install"> 会先行触发重定向。条件收敛（相等即不再 setState），不会渲染循环。
  const cached = peekInstalled()
  if (cached !== null && !(state.status === "ready" && state.installed === cached)) {
    setState({ status: "ready", installed: cached })
  }

  useEffect(() => {
    if (peekInstalled() !== null) return
    let cancelled = false
    checkInstalled()
      .then((installed) => {
        if (!cancelled) setState({ status: "ready", installed })
      })
      .catch(() => {
        if (!cancelled) setState({ status: "error" })
      })
    return () => {
      cancelled = true
    }
  }, [pathname])

  const retry = () => {
    setState({ status: "loading" })
    checkInstalled(true)
      .then((installed) => setState({ status: "ready", installed }))
      .catch(() => setState({ status: "error" }))
  }

  if (state.status === "loading") return <FullPageSpinner label="正在检查安装状态…" />
  if (state.status === "error") return <BackendDownScreen onRetry={retry} />
  if (!state.installed && pathname !== "/install") return <Navigate to="/install" replace />
  if (state.installed && pathname === "/install") return <Navigate to="/" replace />
  return <Outlet />
}
