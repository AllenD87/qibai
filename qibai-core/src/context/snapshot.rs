//! 每次运行的上下文快照（方案 5.2）。
//!
//! 一次 `run` 写一份 `context/<run_id>.json`：
//! - 四层渲染后的文本（L0/L1/L2/L3）；
//! - 注入的技能名、命中的 KB chunk id；
//! - token 估算总量；
//! - `prompt_version` = 模板规范化后的 hash；
//! - `parent_run_id` 链完整，能一路回溯到原始 message id。
//!
//! 快照只写本地，不上传。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::context::compress::CompressionState;
use crate::context::layers::KbHit;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextSnapshot {
    pub run_id: String,
    /// 上一次运行的快照 id；首条为 None。
    pub parent_run_id: Option<String>,
    /// 四层渲染后的文本（用于面板"这一步为什么这么做"展开）。
    pub l0: String,
    pub l1: String,
    pub l2: String,
    pub l3: String,
    /// 本步注入的技能名（含全文加载）。
    pub skills_injected: Vec<String>,
    /// KB 命中片段。
    pub kb_hits: Vec<KbHit>,
    /// 估算 token 总量（不含 reserve）。
    pub tokens: usize,
    /// 提示词模板规范化 hash（前 16 hex）。
    pub prompt_version: String,
    /// 本次压缩状态。
    pub compression: CompressionState,
    /// 本次被压成摘要的原始 message id 列表（parent_snapshot 链的核心）。
    pub dropped_message_ids: Vec<String>,
    /// 本次被淘汰进 archive 的事实卡 id（可回查）。
    pub archived_fact_ids: Vec<String>,
}

impl ContextSnapshot {
    /// 读回某份快照（用于 parent_snapshot 回溯）。
    pub fn read(dir: &Path, run_id: &str) -> Result<Self, std::io::Error> {
        let p = snapshot_path(dir, run_id);
        let txt = std::fs::read_to_string(&p)?;
        let s: Self = serde_json::from_str(&txt)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(s)
    }
}

fn snapshot_path(dir: &Path, run_id: &str) -> PathBuf {
    dir.join(format!("{}.json", run_id))
}

/// 写快照到 `dir/<run_id>.json`（原子写：tmp + rename）。
pub fn write_snapshot(dir: &Path, snap: &ContextSnapshot) -> Result<PathBuf, std::io::Error> {
    std::fs::create_dir_all(dir)?;
    let final_path = snapshot_path(dir, &snap.run_id);
    let tmp_path = dir.join(format!("{}.json.tmp", snap.run_id));
    let txt = serde_json::to_string_pretty(snap)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(&tmp_path, txt)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(final_path)
}

/// 计算 prompt_version：把 L0/L1 目录清单/L2 模板规范化后 sha256，取前 16 hex。
///
/// 规范化规则：压缩连续空白为单空格、去 BOM、去行尾空格、按行排序——
/// 只要模板"语义相同"，hash 一致；模板改一行，hash 必变。
pub fn prompt_version(l0: &str, l1_catalog: &str, l2: &str) -> String {
    let normalized = normalize(l0, l1_catalog, l2);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    digest.iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

fn normalize(parts: &str, catalog: &str, brief: &str) -> String {
    use std::collections::BTreeSet;
    let mut set = BTreeSet::new();
    for chunk in [parts, catalog, brief] {
        for line in chunk.split('\n') {
            let trimmed = line.trim().trim_end();
            if trimmed.is_empty() { continue; }
            // 折叠连续空白
            let folded: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
            set.insert(folded);
        }
    }
    set.into_iter().collect::<Vec<_>>().join("\n")
}

/// 从起始 run_id 回溯整条快照链，返回完整链路（从最早到当前）。
/// 断链时返回当前已读到的部分 + 最后一个可读 run_id，不 panic。
pub fn walk_chain(dir: &Path, mut run_id: String) -> (Vec<ContextSnapshot>, Option<String>) {
    let mut chain: Vec<ContextSnapshot> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(run_id.clone()) {
            // 环路保护
            return (chain, Some(run_id));
        }
        match ContextSnapshot::read(dir, &run_id) {
            Ok(snap) => {
                let parent = snap.parent_run_id.clone();
                chain.push(snap);
                match parent {
                    Some(p) if !p.is_empty() => run_id = p,
                    _ => return (chain, None),
                }
            }
            Err(e) => {
                return (chain, Some(format!("{} (read error: {})", run_id, e)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snap(run_id: &str, parent: Option<&str>, dropped: Vec<&str>) -> ContextSnapshot {
        ContextSnapshot {
            run_id: run_id.into(),
            parent_run_id: parent.map(|s| s.into()),
            l0: "L0".into(),
            l1: "L1".into(),
            l2: "L2".into(),
            l3: "L3".into(),
            skills_injected: vec!["web".into()],
            kb_hits: vec![],
            tokens: 123,
            prompt_version: prompt_version("L0", "L1", "L2"),
            compression: CompressionState::Compressed,
            dropped_message_ids: dropped.into_iter().map(|s| s.into()).collect(),
            archived_fact_ids: vec![],
        }
    }

    #[test]
    fn prompt_version_stable_for_same_template() {
        let v1 = prompt_version("你是小小", "技能列表", "目标：demo");
        let v2 = prompt_version("  你是小小  \n", "技能列表", "目标：demo");
        assert_eq!(v1, v2, "规范化后应相同");
        assert_eq!(v1.len(), 16);
    }

    #[test]
    fn prompt_version_changes_on_one_line_edit() {
        let v1 = prompt_version("你是小小", "技能列表", "目标：demo");
        let v2 = prompt_version("你是小小", "技能列表", "目标：demo2");
        assert_ne!(v1, v2, "改一行 hash 必变");
    }

    #[test]
    fn write_and_read_snapshot_roundtrip() {
        let dir = std::env::temp_dir().join(format!("qibai_snap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let snap = make_snap("run_001", None, vec!["m0", "m1"]);
        write_snapshot(&dir, &snap).unwrap();
        let back = ContextSnapshot::read(&dir, "run_001").unwrap();
        assert_eq!(back.run_id, "run_001");
        assert_eq!(back.dropped_message_ids, vec!["m0".to_string(), "m1".to_string()]);
        assert!(back.parent_run_id.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parent_chain_walks_back_to_original_message() {
        let dir = std::env::temp_dir().join(format!("qibai_chain_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // 构造 3 次压缩的链：run3 → run2 → run1（root）
        let s1 = make_snap("run_001", None, vec!["m0", "m1", "m2"]);
        let s2 = make_snap("run_002", Some("run_001"), vec!["m3", "m4"]);
        let s3 = make_snap("run_003", Some("run_002"), vec!["m5"]);
        write_snapshot(&dir, &s1).unwrap();
        write_snapshot(&dir, &s2).unwrap();
        write_snapshot(&dir, &s3).unwrap();

        let (chain, tail) = walk_chain(&dir, "run_003".into());
        assert!(tail.is_none(), "完整链不应断：{:?}", tail);
        assert_eq!(chain.len(), 3);
        // chain 顺序：[run3, run2, run1]
        assert_eq!(chain[0].run_id, "run_003");
        assert_eq!(chain[2].run_id, "run_001");

        // 把所有 dropped_message_ids 汇总，应能回溯到 m0
        let all_dropped: Vec<&str> = chain.iter()
            .flat_map(|s| s.dropped_message_ids.iter().map(|x| x.as_str()))
            .collect();
        assert!(all_dropped.contains(&"m0"), "回溯不到原始 m0：{:?}", all_dropped);
        assert!(all_dropped.contains(&"m5"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn walk_chain_broken_link_does_not_panic() {
        let dir = std::env::temp_dir().join(format!("qibai_chain_broken_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s2 = make_snap("run_002", Some("run_missing"), vec!["m3"]);
        write_snapshot(&dir, &s2).unwrap();
        let (chain, tail) = walk_chain(&dir, "run_002".into());
        assert_eq!(chain.len(), 1);
        assert!(tail.is_some(), "应报告断链");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
