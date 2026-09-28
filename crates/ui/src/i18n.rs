//! Runtime UI language. `t!`/`tf!` resolve the active locale from a
//! thread-local set once per render (`set_lang` in `Shell::view`); keys are
//! the zh literals already used in the code, so a missing table entry falls
//! back to Chinese instead of blanking out. English templates keep `{}` /
//! `{name}` placeholder *order* identical so `tf!` can substitute
//! positionally.
//!
//! Design note: core-side formatters that emit zh text (`Range::label`,
//! `quota_kind_label`, `已过期`) are wrapped with `tr` at the UI call site;
//! `compact()` swaps 万/亿 for K/M/B because suffix math can't be retexted.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    Zh,
    En,
}

impl Lang {
    pub fn from_config(s: &str) -> Self {
        match s {
            "en" => Self::En,
            _ => Self::Zh,
        }
    }
}

/// Render-locale — a plain atomic (not thread-local) because D2D draw
/// closures may run on whichever thread the compositor paints on.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(l: Lang) {
    CURRENT.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::En,
        _ => Lang::Zh,
    }
}

/// Compact token count honoring the active locale: 万/亿 in zh, K/M/B in en.
pub fn compact(n: u64) -> String {
    if lang() == Lang::Zh {
        return globaltokentracker_core::viewmodel::fmt::tokens_compact(n);
    }
    let trim = |v: f64| {
        let s = format!("{v:.1}");
        s.trim_end_matches(".0").to_string()
    };
    if n >= 1_000_000_000 {
        format!("{}B", trim(n as f64 / 1e9))
    } else if n >= 1_000_000 {
        format!("{}M", trim(n as f64 / 1e6))
    } else if n >= 10_000 {
        format!("{}K", trim(n as f64 / 1e3))
    } else {
        globaltokentracker_core::viewmodel::fmt::tokens_exact(n)
    }
}

/// Positional template fill — every `{…}` group (empty or named) consumes the
/// next argument in order. Table entries preserve placeholder order.
pub fn tf(tpl: &str, args: &[&dyn std::fmt::Display]) -> String {
    let tpl = tr(tpl);
    let mut out = String::with_capacity(tpl.len() + 16);
    let mut rest = tpl;
    let mut i = 0;
    while let Some(open) = rest.find('{') {
        let Some(close_rel) = rest[open..].find('}') else {
            break;
        };
        let close = open + close_rel;
        out.push_str(&rest[..open]);
        if let Some(arg) = args.get(i) {
            out.push_str(&arg.to_string());
        } else {
            out.push_str(&rest[open..=close]);
        }
        i += 1;
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// zh literal → English. Unknown keys pass through as Chinese (never blank).
/// Generic over lifetime so runtime-produced strings (`fmt::until`, quota
/// kind labels) translate alongside literals.
pub fn tr(zh: &str) -> &str {
    if lang() == Lang::Zh {
        return zh;
    }
    match zh {
        // ---- nav & page titles
        "总览" => "Overview",
        "明细" => "Details",
        "配额" => "Quota",
        "数据源" => "Sources",
        "价格" => "Prices",
        "价目表（$/1M tokens）" => "Price list ($/1M tokens)",
        "设置" => "Settings",
        // ---- overview
        "{rl} Tokens" => "{rl} Tokens",
        "{rl}估算成本" => "{rl} est. cost",
        "{rl}缓存读" => "{rl} cache reads",
        "{rl}事件" => "{rl} events",
        "{rl}趋势" => "{rl} trend",
        "{rl} · 按工具" => "{rl} · by tool",
        "事件 {}" => "{} events",
        "全部 {}" => "All-time {}",
        "输入 {}" => "Input {}",
        "活跃 {}" => "Active {}",
        "今日 · 按小时" => "Today · hourly",
        "全部 · 按天（近 60 桶）" => "All · daily (last 60 buckets)",
        "暂无数据" => "No data yet",
        "暂无配额信号" => "No quota signals",
        "订阅配额" => "Subscription quota",
        "未计价模型" => "Unpriced models",
        "{} — 请在价格页补充覆写" => "{} — add overrides on the Prices page",
        "估算" => "Est.",
        "上移" => "Move up",
        "下移" => "Move down",
        "隐藏" => "Hide",
        "恢复" => "Restore",
        "已隐藏：{}" => "Hidden: {}",
        "完成" => "Done",
        "布局" => "Layout",
        "刷新" => "Refresh",
        "正在扫描数据源…" => "Scanning sources…",
        // ---- filter chrome
        "工具" => "Tools",
        "模型" => "Models",
        "工具筛选" => "Tool filter",
        "模型筛选" => "Model filter",
        "刷新频率" => "Refresh cadence",
        "全部" => "All",
        "今日" => "Today",
        "近 7 天" => "Last 7 days",
        "近 30 天" => "Last 30 days",
        "自定义" => "Custom",
        "自定义 · 按天" => "Custom · by day",
        "从" => "From",
        "至" => "to",
        "起始日期" => "Start date",
        "截止日期" => "End date",
        "未选" => "None",
        "已选 {}/{total}" => "{}/{total} selected",
        "全选" => "Select all",
        "清空" => "Clear",
        "仅文件变更" => "File changes only",
        "10 秒" => "10 s",
        "30 秒" => "30 s",
        "1 分钟" => "1 min",
        "5 分钟" => "5 min",
        // ---- detail page
        "时间" => "Time",
        "输入" => "Input",
        "输出" => "Output",
        "缓存" => "Cache",
        "缓存读" => "Cache read",
        "缓存写" => "Cache write",
        "成本" => "Cost",
        "时长" => "Duration",
        "来源" => "Source",
        "← 上一页" => "← Prev",
        "下一页 →" => "Next →",
        "第 {} / {} 页 · 共 {} 条" => "Page {} / {} · {} entries",
        "{}  ·  {} 事件" => "{}  ·  {} events",
        "{} tok  ·  {}" => "{} tok  ·  {}",
        "{} tok · {}" => "{} tok · {}",
        "{} · {} tok" => "{} · {} tok",
        "第 {} / {} 项" => "#{} of {}",
        "{} · {}" => "{} · {}",
        "{} · {} 项配额" => "{} · {} quotas",
        "{}…" => "{}…",
        // ---- quota page
        "配额组 {}" => "Quota group {}",
        "{}  ·  reset {}" => "{}  ·  reset {}",
        "余额 {}" => "Balance {}",
        "已用 {} / 上限 {}" => "Used {} / limit {}",
        "用量 {}" => "Used {}",
        "上限 {}" => "Limit {}",
        "重置 {} · 采集 {}" => "Resets {} · collected {}",
        "暂无配额数据" => "No quota data",
        "5 小时窗口" => "5-hour window",
        "每周限额" => "Weekly limit",
        "每月限额" => "Monthly limit",
        "剩余点数" => "Credits",
        "计费周期" => "Billing period",
        "Auto 用量池" => "Auto pool",
        "API 用量池" => "API pool",
        "会话上下文" => "Session context",
        "已过期" => "expired",
        // ---- sources page
        "尚未扫描" => "Not scanned yet",
        "正常" => "OK",
        "⚠ {e}" => "⚠ {e}",
        "{} 文件 · 累计 {} 行 · 游标 {} · 上次 {}" => {
            "{} files · {} rows ingested · {} cursors · last {}"
        }
        // ---- prices page
        "{} 个模型 · 前 400 条 · {}" => "{} models · first 400 · {}",
        "联网同步于 {} 小时前" => "Synced {}h ago",
        "仅本地种子，尚未联网同步" => "Local seed only; not synced yet",
        // ---- tray
        "GlobalTokenTracker — 今日 {}" => "GlobalTokenTracker — today {}",
        "显示 GlobalTokenTracker" => "Show GlobalTokenTracker",
        "隐藏到托盘" => "Hide to tray",
        "退出" => "Quit",
        // ---- widget registry (widgets.rs titles resolve through tr too)
        "统计卡" => "Stats",
        "近 30 天趋势" => "30-day trend",
        "占比分布" => "Share breakdown",
        "费用" => "Cost",
        "按模型" => "By model",
        "按工具" => "By tool",
        "其他" => "Other",
        "本周 · 按工具" => "This week · by tool",
        "未计价提示" => "Unpriced notice",
        "部件" => "Widget",
        // ---- settings page
        "外观" => "Appearance",
        "主题模式" => "Theme",
        "跟随系统" => "System",
        "浅色" => "Light",
        "深色" => "Dark",
        "主题色" => "Accent color",
        "字体" => "Font",
        "界面字号" => "Font size",
        "通用" => "General",
        "语言" => "Language",
        "开机自启动" => "Launch at login",
        "登录 Windows 后自动启动（最小化到托盘）" => {
            "Start with Windows (minimized to tray)"
        }
        "点击关闭按钮时" => "When closing",
        "每次询问" => "Ask every time",
        "彻底退出" => "Quit completely",
        "仅作用于图表文字；界面控件字体跟随系统" => {
            "Applies to chart text; controls follow the system font"
        }
        "主题模式、配色与字号" => "Theme mode, colors and font size",
        "界面语言与启动行为" => "Interface language and startup behavior",
        "默认" => "Default",
        "默认（Segoe UI）" => "Default (Segoe UI)",
        "蓝色" => "Blue",
        "绿色" => "Green",
        "紫色" => "Purple",
        "橙色" => "Orange",
        "红色" => "Red",
        "中文" => "中文",
        // ---- close prompt dialog
        "关闭 GlobalTokenTracker" => "Close GlobalTokenTracker",
        "要彻底退出，还是隐藏到托盘继续后台统计？" => {
            "Quit completely, or keep stats running in the system tray?"
        }
        "记住我的选择" => "Remember my choice",
        "托盘图标不可用" => "Tray icon unavailable",
        "取消" => "Cancel",
        _ => zh,
    }
}

/// `&T → &dyn Display` unsizing helper for `tf!` (cast syntax is brittle here).
#[doc(hidden)]
pub fn as_display<T: std::fmt::Display>(v: &T) -> &dyn std::fmt::Display {
    v
}

/// Literal → localized `&'static str`.
#[macro_export]
macro_rules! t {
    ($s:literal) => {
        $crate::i18n::tr($s)
    };
}

/// Localized format template + positional args (`Display`).
#[macro_export]
macro_rules! tf {
    ($s:literal) => {
        $crate::i18n::tf($s, &[])
    };
    ($s:literal, $($a:expr),+ $(,)?) => {
        $crate::i18n::tf($s, &[$( $crate::i18n::as_display(&$a) ),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Single test — `CURRENT` is a global atomic, parallel cases would race.
    #[test]
    fn locales() {
        set_lang(Lang::Zh);
        assert_eq!(tr("总览"), "总览");
        assert_eq!(tf!("第 {} / {} 页", 1, 3), "第 1 / 3 页");

        set_lang(Lang::En);
        assert_eq!(tr("总览"), "Overview");
        assert_eq!(
            tf!("第 {} / {} 页 · 共 {} 条", 1, 3, 42),
            "Page 1 / 3 · 42 entries"
        );
        assert_eq!(tf!("{rl} · 按工具", "近 7 天"), "近 7 天 · by tool");
        assert_eq!(compact(64_425), "64.4K");
        assert_eq!(compact(1_730_848_235), "1.7B");
        set_lang(Lang::Zh);
    }
}
