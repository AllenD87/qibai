//! v45.13 并行调度器（纯函数 + 单测）。
//!
//! - [`build_groups`]：按 `depends_on` 做拓扑分层，同层超过 `max_parallel` 再切片成子组；
//!   依赖不存在 / 自依赖 / 成环 → [`ScheduleError`]，带涉及的 step id。
//! - [`claimed_writes`]：收集一步声明要写入的路径（`declared_writes` ∪ `artifacts[].path`）。
//! - [`conflicts_within_group`]：组内两两对比，若声明了同一目标路径则记 [`WriteConflict`]。
//! - [`merge_order`]：按组序 + 组内 idx 展平成稳定的 step id 列表（同一输入两次结果一致）。
//!
//! 本模块不依赖 tokio / 存储 / LLM，只吃 [`crate::task::Step`] 切片。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::task::model::Step;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("step `{step}` declares dependency on unknown step `{dep}`")]
    UnknownDependency { step: String, dep: String },
    #[error("step `{step}` depends on itself")]
    SelfDependency { step: String },
    #[error("dependency cycle detected involving steps: {0:?}")]
    Cycle(Vec<String>),
}

/// 同组内两个步骤写入同一路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteConflict {
    pub a: String,
    pub b: String,
    pub path: String,
}

/// 收集一步声明要写入的全部路径（去空白、去重）。
pub fn claimed_writes(step: &Step) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for p in &step.declared_writes {
        let p = p.trim().to_string();
        if !p.is_empty() {
            out.insert(p);
        }
    }
    for a in &step.artifacts {
        let p = a.path.trim().to_string();
        if !p.is_empty() {
            out.insert(p);
        }
    }
    out.into_iter().collect()
}

/// 对组内 step id 两两比对，返回所有写冲突。
pub fn conflicts_within_group(group_ids: &[String], steps: &[Step]) -> Vec<WriteConflict> {
    let by_id: HashMap<&str, &Step> = steps.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut out = Vec::new();
    for i in 0..group_ids.len() {
        for j in (i + 1)..group_ids.len() {
            let a_id = &group_ids[i];
            let b_id = &group_ids[j];
            let (Some(a), Some(b)) = (by_id.get(a_id.as_str()), by_id.get(b_id.as_str())) else {
                continue;
            };
            let wa: BTreeSet<String> = claimed_writes(a).into_iter().collect();
            let wb: BTreeSet<String> = claimed_writes(b).into_iter().collect();
            for p in wa.intersection(&wb) {
                out.push(WriteConflict {
                    a: a_id.clone(),
                    b: b_id.clone(),
                    path: p.clone(),
                });
            }
        }
    }
    out
}

/// 拓扑分层 + max_parallel 切片。
///
/// 返回的 `Vec<Vec<String>>`：外层是"层"（层与层之间有依赖），层内若超过
/// `max_parallel` 会被切成多个子组（子组之间仍同层，只是为了不无限并发）。
/// 组内按 `idx` 升序稳定排序。
pub fn build_groups(steps: &[Step], max_parallel: usize) -> Result<Vec<Vec<String>>, ScheduleError> {
    let max_parallel = max_parallel.max(1);

    // 1. 校验依赖存在性 / 自依赖
    let ids: BTreeSet<&str> = steps.iter().map(|s| s.id.as_str()).collect();
    for s in steps {
        for dep in &s.depends_on {
            if dep == &s.id {
                return Err(ScheduleError::SelfDependency { step: s.id.clone() });
            }
            if !ids.contains(dep.as_str()) {
                return Err(ScheduleError::UnknownDependency {
                    step: s.id.clone(),
                    dep: dep.clone(),
                });
            }
        }
    }

    // 2. 入度表
    let mut indeg: HashMap<&str, usize> = steps.iter().map(|s| (s.id.as_str(), s.depends_on.len())).collect();
    // 反向邻接：dep -> 依赖它的 steps
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for s in steps {
        for dep in &s.depends_on {
            rev.entry(dep.as_str()).or_default().push(s.id.as_str());
        }
    }

    // 3. 分层 Kahn
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut placed: BTreeSet<&str> = BTreeSet::new();

    loop {
        // 当前层：入度为 0 且未放置
        let mut layer: Vec<&Step> = steps
            .iter()
            .filter(|s| !placed.contains(s.id.as_str()) && *indeg.get(s.id.as_str()).unwrap_or(&0) == 0)
            .collect();
        if layer.is_empty() {
            break;
        }
        // 按 idx 升序稳定排序
        layer.sort_by_key(|s| (s.idx, s.id.clone()));

        // 若超过 max_parallel，按 max_parallel 切片成多个子组
        for chunk in layer.chunks(max_parallel) {
            groups.push(chunk.iter().map(|s| s.id.clone()).collect());
        }

        for s in &layer {
            placed.insert(s.id.as_str());
            if let Some(down) = rev.get(s.id.as_str()) {
                for &d in down {
                    let e = indeg.entry(d).or_insert(0);
                    if *e > 0 {
                        *e -= 1;
                    }
                }
            }
        }
    }

    // 4. 还有未放置 → 成环
    if placed.len() != steps.len() {
        let cycle: Vec<String> = steps
            .iter()
            .filter(|s| !placed.contains(s.id.as_str()))
            .map(|s| s.id.clone())
            .collect();
        return Err(ScheduleError::Cycle(cycle));
    }

    Ok(groups)
}

/// 把 groups 展平成 step id 顺序：按组序 + 组内 idx（组内已经在 build_groups 排好序）。
///
/// 为了保证"同样输入两次顺序一致"，即使 groups 由调用方传入，也再按 idx 重排一次。
pub fn merge_order(groups: &[Vec<String>], steps: &[Step]) -> Vec<String> {
    let by_idx: BTreeMap<&str, u32> = steps.iter().map(|s| (s.id.as_str(), s.idx)).collect();
    let mut out = Vec::new();
    for g in groups {
        let mut ids: Vec<&String> = g.iter().collect();
        ids.sort_by_key(|id| (by_idx.get(id.as_str()).copied().unwrap_or(u32::MAX), *id));
        for id in ids {
            out.push(id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, idx: u32, deps: &[&str], writes: &[&str]) -> Step {
        Step {
            id: id.into(),
            idx,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            declared_writes: writes.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn build_groups_linear_chain() {
        let steps = vec![
            step("a", 0, &[], &[]),
            step("b", 1, &["a"], &[]),
            step("c", 2, &["b"], &[]),
        ];
        let g = build_groups(&steps, 3).unwrap();
        assert_eq!(g, vec![vec!["a"], vec!["b"], vec!["c"]]);
    }

    #[test]
    fn build_groups_parallel_layer_then_merge() {
        let steps = vec![
            step("a", 0, &[], &[]),
            step("b", 1, &[], &[]),
            step("c", 2, &[], &[]),
            step("d", 3, &["a", "b", "c"], &[]),
        ];
        let g = build_groups(&steps, 2).unwrap();
        // 第 0 层 [a,b,c] 超过 max=2 → 切成 [a,b] + [c]
        assert_eq!(g[0], vec!["a", "b"]);
        assert_eq!(g[1], vec!["c"]);
        assert_eq!(g[2], vec!["d"]);
    }

    #[test]
    fn build_groups_rejects_unknown_dep() {
        let steps = vec![step("a", 0, &["ghost"], &[])];
        let e = build_groups(&steps, 2).unwrap_err();
        assert!(matches!(e, ScheduleError::UnknownDependency { .. }));
    }

    #[test]
    fn build_groups_rejects_self_dep() {
        let steps = vec![step("a", 0, &["a"], &[])];
        let e = build_groups(&steps, 2).unwrap_err();
        assert!(matches!(e, ScheduleError::SelfDependency { .. }));
    }

    #[test]
    fn build_groups_rejects_cycle() {
        let steps = vec![
            step("a", 0, &["c"], &[]),
            step("b", 1, &["a"], &[]),
            step("c", 2, &["b"], &[]),
        ];
        let e = build_groups(&steps, 2).unwrap_err();
        match e {
            ScheduleError::Cycle(ids) => {
                assert_eq!(ids.len(), 3);
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn claimed_writes_unions_declared_and_artifacts() {
        let s = Step {
            id: "x".into(),
            declared_writes: vec!["a.txt".into(), " ".into()],
            artifacts: vec![crate::task::model::Artifact {
                path: "b.txt".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let w = claimed_writes(&s);
        assert_eq!(w, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn conflicts_within_group_detects_shared_path() {
        let steps = vec![
            step("a", 0, &[], &["out/report.md"]),
            step("b", 1, &[], &["out/report.md"]),
            step("c", 2, &[], &["out/other.md"]),
        ];
        let g = vec!["a".into(), "b".into(), "c".into()];
        let cf = conflicts_within_group(&g, &steps);
        assert_eq!(cf.len(), 1);
        assert_eq!(cf[0].a, "a");
        assert_eq!(cf[0].b, "b");
        assert_eq!(cf[0].path, "out/report.md");
    }

    #[test]
    fn merge_order_is_stable_across_runs() {
        let steps = vec![
            step("a", 0, &[], &[]),
            step("b", 1, &[], &[]),
            step("c", 2, &["a", "b"], &[]),
        ];
        let groups = build_groups(&steps, 2).unwrap();
        let o1 = merge_order(&groups, &steps);
        let o2 = merge_order(&groups, &steps);
        assert_eq!(o1, o2);
        assert_eq!(o1, vec!["a", "b", "c"]);
    }
}
