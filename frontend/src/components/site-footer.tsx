import type { SiteSettings } from "@/lib/types"
import { cn } from "@/lib/utils"

/** 站点页脚：版权行 + 自定义文字（有值时）+ ICP 备案号（有值时，链到工信部备案系统）
 *  wide：wide_layout 主题设置开启（或三列布局）时放宽容器，与主区宽度一致 */
export function SiteFooter({ site, wide = false }: { site: SiteSettings | null; wide?: boolean }) {
  return (
    <footer className="border-t py-8">
      <div
        className={cn(
          "mx-auto flex w-full flex-col items-center gap-1.5 px-4 text-center text-sm text-muted-foreground",
          wide ? "max-w-7xl" : "max-w-5xl",
        )}
      >
        <div>
          © {new Date().getFullYear()} {site?.title || "reedblog"} · Powered by{" "}
          <span className="font-medium">reedblog</span>
        </div>
        {site?.footer_text && (
          <div className="whitespace-pre-line">{site.footer_text}</div>
        )}
        {site?.icp_number && (
          <a
            href="https://beian.miit.gov.cn"
            target="_blank"
            rel="noreferrer"
            className="transition-colors hover:text-foreground hover:underline"
          >
            {site.icp_number}
          </a>
        )}
      </div>
    </footer>
  )
}
