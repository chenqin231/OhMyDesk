//! 最近连接历史：客户端本地 JSON 持久化（跨平台配置目录）。
//!
//! v0 Sentinel 设计的「最近连接」列表数据源。客户端是唯一知道「自己连过谁」的一端，
//! 故历史纯客户端本地维护，不经服务端。每次发起远控（connect_b）记一条，按 id 去重
//! （最近优先），上限 [`MAX_ITEMS`] 条，存到 `<config_dir>/OhMyDesk/recent.json`。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 历史保留上限（超出按时间淘汰最旧）。
const MAX_ITEMS: usize = 8;

/// 一条最近连接记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentConn {
    /// 目标终端 id（已规范化：去空白）。
    pub id: String,
    /// 最近一次连接的 Unix 毫秒时间戳。
    pub ts: i64,
}

/// 历史文件路径：`<config_dir>/OhMyDesk/recent.json`。
/// Linux `~/.config/OhMyDesk/`，Windows `%APPDATA%\OhMyDesk\`。取不到配置目录则返回 None。
fn history_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "OhMyDesk")?;
    Some(dirs.config_dir().join("recent.json"))
}

/// 读取历史（按 ts 倒序）。文件缺失/损坏一律返回空表，不报错（首启动正常）。
pub fn load() -> Vec<RecentConn> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    let mut list: Vec<RecentConn> = serde_json::from_slice(&bytes).unwrap_or_default();
    list.sort_by_key(|c| std::cmp::Reverse(c.ts));
    list.truncate(MAX_ITEMS);
    list
}

/// 记录一次连接：规范化 id → 合并进历史（去重 + 倒序 + 截断）→ 落盘 → 返回最新列表。
/// 落盘失败（无权限等）只忽略，不影响内存返回值与 UI 更新。
pub fn record(raw_id: &str) -> Vec<RecentConn> {
    let id = normalize_id(raw_id);
    if id.is_empty() {
        return load();
    }
    let merged = merge(load(), &id, crate::net::now());
    if let Some(path) = history_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&merged) {
            let _ = std::fs::write(&path, json);
        }
    }
    merged
}

/// 去空白：用户可能输入 "419 027 558"，统一存纯数字便于去重与回填。
pub fn normalize_id(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 纯函数：把 (id, ts) 合并进现有列表——同 id 覆盖为最新时间，按 ts 倒序，截断到上限。
/// 抽出独立函数以便单测（不碰文件系统）。
fn merge(mut list: Vec<RecentConn>, id: &str, ts: i64) -> Vec<RecentConn> {
    list.retain(|c| c.id != id);
    list.insert(0, RecentConn { id: id.to_string(), ts });
    list.sort_by_key(|c| std::cmp::Reverse(c.ts));
    list.truncate(MAX_ITEMS);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 规范化_去除所有空白() {
        assert_eq!(normalize_id("419 027 558"), "419027558");
        assert_eq!(normalize_id("  123\t456 "), "123456");
    }

    #[test]
    fn 合并_同id覆盖为最新且去重() {
        let list = vec![RecentConn { id: "111".into(), ts: 100 }];
        let merged = merge(list, "111", 200);
        assert_eq!(merged.len(), 1, "同 id 不应重复");
        assert_eq!(merged[0].ts, 200, "应覆盖为最新时间");
    }

    #[test]
    fn 合并_倒序且最新置顶() {
        let mut list = vec![RecentConn { id: "a".into(), ts: 100 }];
        list = merge(list, "b", 300);
        assert_eq!(list[0].id, "b", "最新连接应置顶");
        assert_eq!(list[1].id, "a");
    }

    #[test]
    fn 合并_超上限按最旧淘汰() {
        let mut list = Vec::new();
        for i in 0..(MAX_ITEMS as i64 + 3) {
            list = merge(list, &format!("id{i}"), i * 10);
        }
        assert_eq!(list.len(), MAX_ITEMS, "应截断到上限");
        assert_eq!(list[0].id, format!("id{}", MAX_ITEMS as i64 + 2), "最新在顶");
    }
}
