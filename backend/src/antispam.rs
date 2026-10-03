//! 反滥用（契约「反滥用」条款，2026-10-04 新增）：
//! - 评论/留言限流：同 IP + 同目标，60 秒 1 条 + 10 分钟 5 条（阈值常量，超限 429 + Retry-After）
//! - 后台登录失败退避：同 IP + 用户名，连续失败 5 次锁定 15 分钟（锁定期间正确密码也拒绝）
//! - 内容黑名单判定：关键词（大小写不敏感）与正文 URL 数上限（纯函数，供评论创建管线调用）
//!
//! 存储与生命周期（与浏览量去重同风格，见 views.rs）：
//! 进程内 `Mutex<HashMap>`、重启清零、不引入外部依赖；条目达到软上限（MAX_ENTRIES）时
//! 整表清扫过期项，防内存被刷爆。时间源为系统单调时钟。
//!
//! 阈值全部为后端常量；集成测试需要快速触发时用 `AntiSpam::configure` 注入短窗口
//! （仅测试路径调用，生产启动路径从不调用，常量语义不变）。

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

/// 评论限流短窗口：60 秒（契约常量）
pub const COMMENT_SHORT_WINDOW: Duration = Duration::from_secs(60);
/// 评论限流短窗口内条数上限：1 条（契约常量）
pub const COMMENT_SHORT_MAX: usize = 1;
/// 评论限流长窗口：10 分钟（契约常量）
pub const COMMENT_LONG_WINDOW: Duration = Duration::from_secs(10 * 60);
/// 评论限流长窗口内条数上限：5 条（契约常量）
pub const COMMENT_LONG_MAX: usize = 5;
/// 登录连续失败次数上限：5 次（契约常量）
pub const LOGIN_MAX_FAILURES: usize = 5;
/// 登录锁定时长：15 分钟（契约常量）
pub const LOGIN_LOCKOUT: Duration = Duration::from_secs(15 * 60);

/// 条目软上限：超过后在下一次写入前整表清扫过期项（同 views::ViewDedup）
const MAX_ENTRIES: usize = 50_000;

/// 阈值配置（默认 = 契约常量；测试可注入短窗口）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AntiSpamConfig {
    pub comment_short_window: Duration,
    pub comment_short_max: usize,
    pub comment_long_window: Duration,
    pub comment_long_max: usize,
    pub login_max_failures: usize,
    pub login_lockout: Duration,
}

impl Default for AntiSpamConfig {
    fn default() -> Self {
        Self {
            comment_short_window: COMMENT_SHORT_WINDOW,
            comment_short_max: COMMENT_SHORT_MAX,
            comment_long_window: COMMENT_LONG_WINDOW,
            comment_long_max: COMMENT_LONG_MAX,
            login_max_failures: LOGIN_MAX_FAILURES,
            login_lockout: LOGIN_LOCKOUT,
        }
    }
}

/// 进程内反滥用状态：评论限流命中表 + 登录失败表
#[derive(Debug, Default)]
pub struct AntiSpam {
    config: RwLock<AntiSpamConfig>,
    /// key = "{ip}|{target_type}|{target_id}"，value = 窗口内的提交时刻（升序）
    comment_hits: Mutex<HashMap<String, Vec<Instant>>>,
    /// key = "{ip}|{username}"，value = 锁定窗口内的失败时刻（升序）
    login_failures: Mutex<HashMap<String, Vec<Instant>>>,
}

impl AntiSpam {
    pub fn new() -> Self {
        Self::default()
    }

    /// 覆盖阈值（仅供集成测试注入短窗口；生产路径不调用，契约常量语义不变）
    pub fn configure(&self, cfg: AntiSpamConfig) {
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = cfg;
    }

    /// 当前阈值
    pub fn config(&self) -> AntiSpamConfig {
        *self.config.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// 评论限流判定：窗口内未超限 → 记一次命中并 `Ok(())`；
    /// 超限 → `Err(retry_after_secs)`（最早可再次提交的剩余秒数，≥1）。
    /// 判定在内容校验（黑名单/链接数）之后调用，蜜罐与 403 不占用额度（契约判定顺序）。
    pub fn check_and_record_comment(&self, key: &str) -> Result<(), u64> {
        let cfg = self.config();
        let now = Instant::now();
        let mut map = self
            .comment_hits
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if map.len() >= MAX_ENTRIES {
            map.retain(|_, hits| prune(hits, now, cfg.comment_long_window) > 0);
        }
        let hits = map.entry(key.to_string()).or_default();
        prune(hits, now, cfg.comment_long_window);

        // 短/长两个窗口分别计数；任一达到上限即拒绝，剩余秒数取两者中更晚释放的
        let short = hits
            .iter()
            .filter(|t| now.duration_since(**t) < cfg.comment_short_window)
            .count();
        let long = hits.len();
        let mut retry = Duration::ZERO;
        if short >= cfg.comment_short_max {
            retry = retry.max(release_after(
                hits,
                cfg.comment_short_max,
                cfg.comment_short_window,
                now,
            ));
        }
        if long >= cfg.comment_long_max {
            retry = retry.max(release_after(
                hits,
                cfg.comment_long_max,
                cfg.comment_long_window,
                now,
            ));
        }
        if short >= cfg.comment_short_max || long >= cfg.comment_long_max {
            return Err(retry_secs(retry));
        }
        hits.push(now);
        Ok(())
    }

    /// 登录锁定判定：处于锁定期 → `Some(剩余秒数)`；否则 `None`（顺带清理过期失败记录）。
    /// 只在密码校验之前调用（锁定期间即使密码正确也拒绝，契约「反滥用」）
    pub fn login_locked_for(&self, key: &str) -> Option<u64> {
        let cfg = self.config();
        let now = Instant::now();
        let mut map = self
            .login_failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let hits = map.get_mut(key)?;
        prune(hits, now, cfg.login_lockout);
        if hits.is_empty() {
            map.remove(key);
            return None;
        }
        if hits.len() >= cfg.login_max_failures {
            let last = *hits.last().expect("列表非空");
            return Some(retry_secs(
                cfg.login_lockout.saturating_sub(now.duration_since(last)),
            ));
        }
        None
    }

    /// 记录一次登录失败（用户名不存在或密码错误；锁定期间的请求不再计数）
    pub fn record_login_failure(&self, key: &str) {
        let cfg = self.config();
        let now = Instant::now();
        let mut map = self
            .login_failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if map.len() >= MAX_ENTRIES {
            map.retain(|_, hits| prune(hits, now, cfg.login_lockout) > 0);
        }
        let hits = map.entry(key.to_string()).or_default();
        prune(hits, now, cfg.login_lockout);
        hits.push(now);
    }

    /// 登录成功清零该 IP + 用户名的失败计数
    pub fn clear_login_failures(&self, key: &str) {
        self.login_failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}

/// 清理超出窗口的时刻；返回窗口内条数
fn prune(hits: &mut Vec<Instant>, now: Instant, window: Duration) -> usize {
    hits.retain(|t| now.duration_since(*t) < window);
    hits.len()
}

/// 达到上限时，最早「释放一个名额」的时刻距 now 的时长：
/// 需再过期 `len - max + 1` 条，即第 `len - max`（0 基）早的命中过期后窗口内条数
/// 降到 max 以下。`max == 0`（防御性分支，契约常量为 1/5）按整窗口处理。
fn release_after(hits: &[Instant], max: usize, window: Duration, now: Instant) -> Duration {
    if hits.is_empty() || max == 0 {
        return window;
    }
    let idx = hits.len() - max.min(hits.len());
    window.saturating_sub(now.duration_since(hits[idx]))
}

/// 剩余时长 → Retry-After 秒数：向上取整、至少 1
fn retry_secs(d: Duration) -> u64 {
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    secs.max(1)
}

// ---------- 内容黑名单（纯函数，供评论创建管线调用） ----------

/// 解析关键词黑名单设置：按换行或逗号分隔，trim、丢弃空项、统一小写（大小写不敏感匹配）。
/// 契约「反滥用」：`comment_blocked_keywords`
pub fn parse_blocked_keywords(raw: &str) -> Vec<String> {
    raw.split(['\n', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// 正文 URL 数（契约口径：`http://` / `https://` 出现次数，大小写不敏感）
pub fn count_links(content: &str) -> usize {
    let lower = content.to_ascii_lowercase();
    lower.matches("http://").count() + lower.matches("https://").count()
}

/// 内容是否命中黑名单规则（关键词任一命中，或链接数超过上限；max_links=0 表示不限制）。
/// 命中的具体原因只用于内部判定，绝不进入响应文案（契约「反滥用」）
pub fn content_rejected(keywords: &[String], max_links: i64, content: &str) -> bool {
    if max_links > 0 && count_links(content) as i64 > max_links {
        return true;
    }
    if keywords.is_empty() {
        return false;
    }
    let lower = content.to_lowercase();
    keywords.iter().any(|k| lower.contains(k.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_config() -> AntiSpamConfig {
        AntiSpamConfig {
            comment_short_window: Duration::from_millis(80),
            comment_short_max: 1,
            comment_long_window: Duration::from_millis(400),
            comment_long_max: 2,
            login_max_failures: 3,
            login_lockout: Duration::from_millis(150),
        }
    }

    #[test]
    fn default_config_matches_contract_constants() {
        let cfg = AntiSpamConfig::default();
        assert_eq!(cfg.comment_short_window, Duration::from_secs(60));
        assert_eq!(cfg.comment_short_max, 1);
        assert_eq!(cfg.comment_long_window, Duration::from_secs(600));
        assert_eq!(cfg.comment_long_max, 5);
        assert_eq!(cfg.login_max_failures, 5);
        assert_eq!(cfg.login_lockout, Duration::from_secs(900));
    }

    #[test]
    fn comment_rate_limit_blocks_and_recovers() {
        let a = AntiSpam::new();
        a.configure(fast_config());
        assert!(a.check_and_record_comment("1.1.1.1|post|1").is_ok());
        // 短窗口内第二条被拒（即使长窗口还有额度）
        let retry = a.check_and_record_comment("1.1.1.1|post|1").unwrap_err();
        assert!(retry >= 1, "Retry-After 至少 1 秒");
        // 不同目标互不影响
        assert!(a.check_and_record_comment("1.1.1.1|post|2").is_ok());
        assert!(a.check_and_record_comment("2.2.2.2|post|1").is_ok());
        // 短窗口过期后可再发（此时长窗口内已 2 条，达到长窗口上限）
        std::thread::sleep(Duration::from_millis(100));
        assert!(a.check_and_record_comment("1.1.1.1|post|1").is_ok());
        // 再过短窗口，短窗口不拦，但长窗口（max=2）仍拦下第 3 条
        std::thread::sleep(Duration::from_millis(100));
        let err = a.check_and_record_comment("1.1.1.1|post|1").unwrap_err();
        assert!(err >= 1);
    }

    #[test]
    fn login_backoff_locks_after_max_failures_and_clears() {
        let a = AntiSpam::new();
        a.configure(fast_config());
        let key = "1.1.1.1|admin";
        assert_eq!(a.login_locked_for(key), None);
        for _ in 0..3 {
            a.record_login_failure(key);
        }
        // 第 3 次失败后进入锁定（测试配置 3 次），剩余时间 ≥1 秒
        assert!(a.login_locked_for(key).unwrap() >= 1);
        // 其他 IP / 用户名不受影响
        assert_eq!(a.login_locked_for("2.2.2.2|admin"), None);
        assert_eq!(a.login_locked_for("1.1.1.1|other"), None);
        // 锁定期满自动解锁（失败窗口过期）
        std::thread::sleep(Duration::from_millis(170));
        assert_eq!(a.login_locked_for(key), None);
        // 成功清零
        a.record_login_failure(key);
        a.record_login_failure(key);
        assert_eq!(a.login_locked_for(key), None);
        a.clear_login_failures(key);
        a.record_login_failure(key);
        a.record_login_failure(key);
        assert_eq!(a.login_locked_for(key), None, "清零后需重新累计到上限");
    }

    #[test]
    fn keyword_parsing_and_content_check() {
        let kws = parse_blocked_keywords(" spam \n广告, Viagra ,,\n\n 赌博 ");
        assert_eq!(kws, vec!["spam", "广告", "viagra", "赌博"]);
        assert!(content_rejected(&kws, 0, "这里有 SPAM 字样"));
        assert!(content_rejected(&kws, 0, "含 viagra 的正文"));
        assert!(!content_rejected(&kws, 0, "正常评论"));
        // 链接数上限：0 = 不限制
        let links = "a http://x.com b https://y.com c https://z.com";
        assert_eq!(count_links(links), 3);
        assert_eq!(count_links("HTTP://X.COM"), 1, "大小写不敏感");
        assert!(content_rejected(&[], 2, links));
        assert!(!content_rejected(&[], 3, links));
        assert!(!content_rejected(&[], 0, links), "0 表示不限制");
        // 空关键词串解析为空列表
        assert!(parse_blocked_keywords("  \n , ").is_empty());
    }
}
