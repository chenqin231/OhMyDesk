//! ui_glue 纯工具：id 分组、相对时间、Slint model 构造、路径运算、聊天行追加。
//! 全部无副作用，可单测。

use crate::{history, FileEntry, HistoryItem};
use slint::{ModelRc, VecModel};

/// 把 9 位 id 按 3-3-3 分组展示（"617343065" → "617 343 065"）。复制时 Rust 侧再去空白。
pub fn group_digits(id: &str) -> String {
    let digits: String = id.chars().filter(|c| !c.is_whitespace()).collect();
    digits
        .as_bytes()
        .chunks(3)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 相对时间（毫秒）：刚刚 / N 分钟前 / N 小时前 / N 天前。
pub fn rel_time(ts_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - ts_ms).max(0) / 1000;
    if secs < 60 {
        "刚刚".into()
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else if secs < 86_400 {
        format!("{} 小时前", secs / 3600)
    } else {
        format!("{} 天前", secs / 86_400)
    }
}

/// 把历史记录构造为 Slint 列表模型（必须在 UI 线程调用：VecModel 非 Send）。
pub fn build_history_model(items: &[history::RecentConn], now_ms: i64) -> ModelRc<HistoryItem> {
    let rows: Vec<HistoryItem> = items
        .iter()
        .map(|c| HistoryItem {
            raw_id: c.id.clone().into(),
            label: group_digits(&c.id).into(),
            sub: rel_time(c.ts, now_ms).into(),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// 把 protocol::FileEntry 列表构造为 Slint 列表模型（必须在 UI 线程调用）。
pub fn build_file_model(items: &[protocol::FileEntry]) -> ModelRc<FileEntry> {
    let rows: Vec<FileEntry> = items
        .iter()
        .map(|e| FileEntry {
            name: e.name.clone().into(),
            is_dir: e.is_dir,
            // u64→i32：仅展示用，超 i32 的文件大小极少见，饱和截断不影响功能。
            size: e.size.min(i32::MAX as u64) as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// 解析 Slint 传来的路径指令串 → 目标绝对路径。
/// "<up>:当前路径" → 父目录；"<cd>:当前路径|子名" → 子目录；其余原样（首次/直填路径）。
pub fn resolve_path_arg(arg: &str, cur: &str) -> String {
    if let Some(rest) = arg.strip_prefix("<up>:") {
        parent_of(rest)
    } else if let Some(rest) = arg.strip_prefix("<cd>:") {
        match rest.split_once('|') {
            Some((base, name)) => join_path(base, name),
            None => cur.to_string(),
        }
    } else {
        arg.to_string()
    }
}

/// 父目录：去掉最后一段；到顶（无分隔或仅根）返回空串（被控端空路径=home/盘符列表）。
pub fn parent_of(path: &str) -> String {
    let win = path.contains('\\');
    let sep = if win { '\\' } else { '/' };
    let trimmed = path.trim_end_matches(sep);
    match trimmed.rsplit_once(sep) {
        // head 为空（如 "/home" 的父）→ 空（回根列表）
        Some(("", _)) => String::new(),
        // Windows 盘根 "C:" → 保留 "C:\"（回盘根而非此电脑）
        Some((head, _)) if win && head.ends_with(':') => format!("{head}{sep}"),
        Some((head, _)) => head.to_string(),
        // 无分隔符（如 "C:" 去尾后无 '\'）→ 空（回此电脑）
        None => String::new(),
    }
}

/// 拼接目录 + 子名（按 base 是否含 '\' 选分隔符）。base 为空时返回 name 本身。
pub fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        return name.to_string();
    }
    let win = base.contains('\\');
    let sep = if win { '\\' } else { '/' };
    let base = base.trim_end_matches(sep);
    format!("{base}{sep}{name}")
}

/// 聊天记录追加一行（"发送者: 文本"），保持纯文本累积（Slint Text 渲染）。
pub fn append_line(log: &str, who: &str, text: &str) -> String {
    if log.is_empty() {
        format!("{who}: {text}")
    } else {
        format!("{log}\n{who}: {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 路径父级_unix与windows() {
        assert_eq!(parent_of("/home/me/docs"), "/home/me");
        assert_eq!(parent_of("/home"), "");
        assert_eq!(parent_of(r"C:\Users\me"), r"C:\Users");
        assert_eq!(parent_of(r"C:\"), ""); // 盘根回此电脑
    }

    #[test]
    fn 路径拼接_按分隔符() {
        assert_eq!(join_path("/home/me", "a.txt"), "/home/me/a.txt");
        assert_eq!(join_path(r"C:\Users", "a.txt"), r"C:\Users\a.txt");
        assert_eq!(join_path("", "a.txt"), "a.txt");
    }

    #[test]
    fn 指令串解析_up与cd() {
        assert_eq!(resolve_path_arg("<up>:/home/me/docs", ""), "/home/me");
        assert_eq!(resolve_path_arg("<cd>:/home/me|docs", ""), "/home/me/docs");
        assert_eq!(resolve_path_arg("/etc", "/home"), "/etc"); // 直填原样
    }

    #[test]
    fn 聊天行追加() {
        assert_eq!(append_line("", "我", "hi"), "我: hi");
        assert_eq!(append_line("我: hi", "对方", "yo"), "我: hi\n对方: yo");
    }
}
