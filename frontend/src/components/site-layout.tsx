import { Outlet } from "react-router-dom"

import { SiteFooter } from "@/components/site-footer"
import { SiteHeader } from "@/components/site-header"
import { useSite } from "@/hooks/use-site"

export function SiteLayout() {
  const { site } = useSite()

  return (
    <div className="flex min-h-screen flex-col bg-background">
      <SiteHeader site={site} />
      <main className="mx-auto w-full max-w-5xl flex-1 px-4 py-8">
        <Outlet />
      </main>
      <SiteFooter site={site} />
    </div>
  )
}
