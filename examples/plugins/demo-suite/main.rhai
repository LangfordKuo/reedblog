// reedblog 示例插件「博客小套件」入口脚本（Rhai，沙箱内执行）。
//
// 钩子函数名 = 钩子 id 把 `.` 换成 `_`，且必须在 manifest.toml 的 hooks 中声明：
//   post_before_render(ctx)    文章渲染前，可改写 title / content_md
//   post_after_render(ctx)     文章渲染后，可改写 content_html
//   comment_before_create(ctx) 评论创建前，放行 / 拦截 / 改写字段
//   post_after_publish(ctx)    文章发布后通知（返回值被忽略）
//
// 沙箱限制：调用深度 64、操作数 50_000、字符串 1MB、数组/映射各 10000 项，
// 无文件 / 网络 / 进程能力。print() 会输出到后端进程的 stdout，可用于调试。
// 除 comment.before_create 外，钩子必须返回 map；运行时出错会被跳过并记入
// 插件详情的 last_error 字段，不会阻断主流程。

// ---------- comment.before_create：评论违禁词过滤 ----------
fn comment_before_create(ctx) {
    // ctx 字段：post_slug / author_name / email（未填时为空字符串）/ content
    let banned = ["spam", "赌场", "代开发票", "加微信", "click here"];

    let content = ctx.content.to_lower();
    let author = ctx.author_name.to_lower();

    for word in banned {
        if content.contains(word) {
            print("demo-suite 拦截了 " + ctx.post_slug + " 的评论：命中违禁词 " + word);
            // 返回 block → 评论创建被拒绝（HTTP 403 comment_blocked），
            // reason 会原样进入响应的 error.message
            return #{ action: "block", reason: "评论包含违禁内容，已拒绝发布" };
        }
        if author.contains(word) {
            return #{ action: "block", reason: "昵称包含违禁内容，已拒绝发布" };
        }
    }

    // 放行。action 必须为 "allow"；还可携带 author_name / email / content
    // 改写评论字段（下一个链上插件会看到改写后的值），例如：
    //   #{ action: "allow", author_name: ctx.author_name + "（已审核）" }
    #{ action: "allow" }
}

// ---------- post.after_render：文章尾部追加版权声明 ----------
fn post_after_render(ctx) {
    // ctx 字段：title / content_html（Markdown 已渲染成的 HTML）/ slug
    let footer = "<footer class=\"demo-suite-copyright\" style=\"margin-top:2rem;padding-top:1rem;border-top:1px solid var(--border);font-size:0.875rem;color:var(--muted-foreground);\">";
    footer += "本文《" + ctx.title + "》采用 CC BY-NC 4.0 许可协议，转载请注明出处。";
    footer += "</footer>";

    ctx.content_html += footer;
    ctx // 必须返回 map；后端只取其中的 content_html 作为最终 HTML
}
