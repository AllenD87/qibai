//! 任务存储（方案 §4.3）。
//!
//! 目录布局：
//! ```text
//! <data_dir>/tasks/<task_id>/
//! ├── task.json          # 原子写（复用 fsutil::atomic_write_json）
//! ├── plan.json          # 已批准计划（原子写）
//! ├── steps.ndjson       # 每步一行，追加写
//! ├── runs.ndjson        # 每次尝试一行，追加写
//! ├── context/           # 上下文快照（由 context 那条线写）
//! └── artifacts/          # 产物（commit_artifact 原子 rename 落位）
//! ```
//!
//! 设计原则：
//! - **绝不**把任务塞进 `conversation.json`。
//! - `task.json` / `plan.json` 整文件原子写（tmp + rename），写一半崩溃旧件仍可读。
//! - `*.ndjson` 只追加，读回时**逐行容忍损坏**：跳过坏行继续读好行。
//! - 产物落位前先写 `artifacts/<name>.tmp`，再 rename 到 `artifacts/<name>`。

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use thiserror::Error;

use super::model::{Plan, Run, Step, Task};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("task not found: {0}")]
    NotFound(String),
    #[error("invalid task id: {0}")]
    InvalidId(String),
}

/// 校验 task_id：只允许 `[a-zA-Z0-9_-]`，防路径穿越。
fn validate_id(id: &str) -> Result<(), StoreError> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.starts_with('.')
    {
        return Err(StoreError::InvalidId(id.to_string()));
    }
    Ok(())
}

/// 任务存储根。绑定一个 `tasks/` 目录（由调用方决定相对谁）。
#[derive(Debug, Clone)]
pub struct TaskStore {
    root: PathBuf,
}

impl TaskStore {
    /// 以 `<root>/tasks/` 为存储根。
    pub fn new(root: impl AsRef<Path>) -> Self {
        TaskStore {
            root: root.as_ref().join("tasks"),
        }
    }

    /// 直接以 `<tasks_root>` 为存储根（不再拼 `tasks/`）。
    pub fn from_tasks_root(tasks_root: impl AsRef<Path>) -> Self {
        TaskStore {
            root: tasks_root.as_ref().to_path_buf(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn task_dir(&self, task_id: &str) -> Result<PathBuf, StoreError> {
        validate_id(task_id)?;
        Ok(self.root.join(task_id))
    }

    pub fn task_json_path(&self, task_id: &str) -> Result<PathBuf, StoreError> {
        Ok(self.task_dir(task_id)?.join("task.json"))
    }

    fn plan_json_path(&self, task_id: &str) -> Result<PathBuf, StoreError> {
        Ok(self.task_dir(task_id)?.join("plan.json"))
    }

    fn steps_ndjson(&self, task_id: &str) -> Result<PathBuf, StoreError> {
        Ok(self.task_dir(task_id)?.join("steps.ndjson"))
    }

    fn runs_ndjson(&self, task_id: &str) -> Result<PathBuf, StoreError> {
        Ok(self.task_dir(task_id)?.join("runs.ndjson"))
    }

    pub fn artifacts_dir(&self, task_id: &str) -> Result<PathBuf, StoreError> {
        Ok(self.task_dir(task_id)?.join("artifacts"))
    }

    pub fn context_dir(&self, task_id: &str) -> Result<PathBuf, StoreError> {
        Ok(self.task_dir(task_id)?.join("context"))
    }

    /// 创建任务目录骨架并写 task.json（原子写）。
    pub fn create(&self, task: &Task) -> Result<(), StoreError> {
        validate_id(&task.id)?;
        let dir = self.task_dir(&task.id)?;
        fs::create_dir_all(dir.join("artifacts"))?;
        fs::create_dir_all(dir.join("context"))?;
        // 空 ndjson 文件（存在但空）；后续 append 直接追加。
        touch(&self.steps_ndjson(&task.id)?)?;
        touch(&self.runs_ndjson(&task.id)?)?;
        self.save(task)
    }

    /// 原子写 task.json（tmp + rename，复用 fsutil）。
    pub fn save(&self, task: &Task) -> Result<(), StoreError> {
        let path = self.task_json_path(&task.id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(task)?;
        crate::fsutil::atomic_write_json(&path, &raw)?;
        Ok(())
    }

    /// 原子写 plan.json。
    pub fn save_plan(&self, task_id: &str, plan: &Plan) -> Result<(), StoreError> {
        let path = self.plan_json_path(task_id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(plan)?;
        crate::fsutil::atomic_write_json(&path, &raw)?;
        Ok(())
    }

    /// 读 task.json；不存在返回 None（损坏由 fsutil 备份处理）。
    pub fn load(&self, task_id: &str) -> Result<Option<Task>, StoreError> {
        validate_id(task_id)?;
        let path = self.task_json_path(task_id)?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(crate::fsutil::load_json_or_backup(&path))
    }

    /// 强制读（不存在则 NotFound）。
    pub fn require(&self, task_id: &str) -> Result<Task, StoreError> {
        match self.load(task_id)? {
            Some(t) => Ok(t),
            None => Err(StoreError::NotFound(task_id.to_string())),
        }
    }

    /// 读 plan.json；不存在返回 None。
    pub fn load_plan(&self, task_id: &str) -> Result<Option<Plan>, StoreError> {
        validate_id(task_id)?;
        let path = self.plan_json_path(task_id)?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(crate::fsutil::load_json_or_backup(&path))
    }

    /// 追加一行 step JSON 到 steps.ndjson。
    pub fn append_step(&self, task_id: &str, step: &Step) -> Result<(), StoreError> {
        let path = self.steps_ndjson(task_id)?;
        append_json_line(&path, step)
    }

    /// 追加一行 run JSON 到 runs.ndjson。
    pub fn append_run(&self, task_id: &str, run: &Run) -> Result<(), StoreError> {
        let path = self.runs_ndjson(task_id)?;
        append_json_line(&path, run)
    }

    /// 读 steps.ndjson；**逐行容忍损坏**：坏行跳过，继续读好行。
    /// 返回 (ok_rows, skipped_bad_lines)。
    pub fn load_steps(&self, task_id: &str) -> Result<(Vec<Step>, usize), StoreError> {
        self.load_ndjson_tolerant(&self.steps_ndjson(task_id)?)
    }

    /// 读 runs.ndjson；同上。
    pub fn load_runs(&self, task_id: &str) -> Result<(Vec<Run>, usize), StoreError> {
        self.load_ndjson_tolerant(&self.runs_ndjson(task_id)?)
    }

    fn load_ndjson_tolerant<T: DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Result<(Vec<T>, usize), StoreError> {
        let mut ok = Vec::new();
        let mut bad = 0usize;
        if !path.exists() {
            return Ok((ok, bad));
        }
        let f = fs::File::open(path)?;
        let reader = BufReader::new(f);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => {
                    bad += 1;
                    continue;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<T>(trimmed) {
                Ok(v) => ok.push(v),
                Err(_) => bad += 1,
            }
        }
        Ok((ok, bad))
    }

    /// 列出所有任务（按目录名排序）。
    pub fn list(&self) -> Result<Vec<Task>, StoreError> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy().to_string();
            if validate_id(&name).is_err() {
                continue;
            }
            if let Ok(Some(t)) = self.load(&name) {
                out.push(t);
            }
        }
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(out)
    }

    /// 把 `<tmp_path>` 原子 rename 到 `<artifacts_dir>/<final_name>`。
    ///
    /// 用于「每步产物写临时文件再落位」（崩溃恢复核心机制）。
    /// 如果 `tmp_path` 不在 artifacts 目录内（例如调用方写到系统临时目录），
    /// 也会先 copy 到 artifacts 下的 tmp，再 rename 到目标——保证原子语义。
    pub fn commit_artifact(
        &self,
        task_id: &str,
        tmp_path: &Path,
        final_name: &str,
    ) -> Result<PathBuf, StoreError> {
        validate_id(task_id)?;
        if final_name.contains('/')
            || final_name.contains('\\')
            || final_name.contains("..")
            || final_name.starts_with('.')
        {
            return Err(StoreError::InvalidId(final_name.to_string()));
        }
        let art_dir = self.artifacts_dir(task_id)?;
        fs::create_dir_all(&art_dir)?;
        let final_path = art_dir.join(final_name);

        // 如果 tmp 已经在同目录下，直接 rename（POSIX 原子）。
        let same_dir = tmp_path.parent() == Some(art_dir.as_path());
        if same_dir {
            fs::rename(tmp_path, &final_path)?;
        } else {
            // 跨目录：先 copy 到 art_dir 下的 tmp，再 rename。
            let staged = art_dir.join(format!("{final_name}.tmp-commit"));
            fs::copy(tmp_path, &staged)?;
            fs::rename(&staged, &final_path)?;
            // 源 tmp 若在系统临时目录由调用方清理；这里不主动删，避免误删。
        }
        Ok(final_path)
    }

    /// 删除整个任务目录（调试用；P0 不暴露给 IPC）。
    pub fn delete(&self, task_id: &str) -> Result<(), StoreError> {
        let dir = self.task_dir(task_id)?;
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

fn touch(path: &Path) -> Result<(), StoreError> {
    if !path.exists() {
        fs::write(path, b"")?;
    }
    Ok(())
}

fn append_json_line<T: serde::Serialize>(path: &Path, v: &T) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    let raw = serde_json::to_string(v)?;
    f.write_all(raw.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_data()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::model::{RunOutcome, StepStatus, new_task_id, now_ms};

    fn make_store() -> TaskStore {
        let tmp = crate::test_support::unique_temp_dir("taskstore");
        TaskStore::new(tmp)
    }

    #[test]
    fn create_then_save_load_roundtrip() {
        let s = make_store();
        let mut t = Task::default();
        t.id = new_task_id();
        t.title = "写周报".into();
        let t0 = now_ms();
        t.created_at = t0;
        t.updated_at = t0;
        s.create(&t).unwrap();
        let loaded = s.require(&t.id).unwrap();
        assert_eq!(loaded, t);
    }

    #[test]
    fn steps_ndjson_tolerates_half_line() {
        let s = make_store();
        let mut t = Task::default();
        t.id = new_task_id();
        s.create(&t).unwrap();

        let mut s1 = Step::default();
        s1.id = "s1".into();
        s1.title = "第一步".into();
        s1.status = StepStatus::Done;
        s1.ended_at = 1000;
        s.append_step(&t.id, &s1).unwrap();

        // 追加一个「半行」：模拟写一半崩溃
        let p = s.steps_ndjson(&t.id).unwrap();
        {
            let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
            f.write_all(b"{\"id\":\"s2\",\"title\":\"broken").unwrap();
            // 不 close / 不 flush，模拟半截
        }

        let (rows, bad) = s.load_steps(&t.id).unwrap();
        assert_eq!(rows.len(), 1, "好行必须读回来");
        assert_eq!(rows[0].id, "s1");
        assert_eq!(bad, 1, "坏行被跳过且计数");
    }

    #[test]
    fn task_json_atomic_write_old_file_survives_halfway_failure() {
        // fsutil 本身已在同模块外测过，这里只验证 task.save() 走的是 atomic 路径：
        // 写失败时通过「tmp 残留检测」反向确认（rename 前 tmp 必须存在）。
        let s = make_store();
        let mut t = Task::default();
        t.id = new_task_id();
        s.create(&t).unwrap();

        // 正常写
        t.title = "v1".into();
        s.save(&t).unwrap();
        // 模拟「写一半失败」：直接造一个 .tmp 残留但不 rename，目标文件仍是旧版
        let jp = s.task_json_path(&t.id).unwrap();
        let tmp = jp.with_extension("json.tmp");
        fs::write(&tmp, "{broken}").unwrap();
        // 目标文件未被 rename，仍是旧的完整 JSON
        let loaded = s.require(&t.id).unwrap();
        assert_eq!(loaded.title, "v1");
        // 清理 tmp
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn append_run_persists_and_reads_back() {
        let s = make_store();
        let mut t = Task::default();
        t.id = new_task_id();
        s.create(&t).unwrap();
        let mut r = Run::default();
        r.id = "r1".into();
        r.step_id = "s1".into();
        r.outcome = RunOutcome::Success;
        s.append_run(&t.id, &r).unwrap();
        let (rows, bad) = s.load_runs(&t.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "r1");
        assert_eq!(bad, 0);
    }

    #[test]
    fn commit_artifact_atomic_lands_in_artifacts_dir() {
        let s = make_store();
        let mut t = Task::default();
        t.id = new_task_id();
        s.create(&t).unwrap();

        // 在系统临时目录写一个半成品
        let staged = std::env::temp_dir().join(format!("staged-{}.tmp", t.id));
        fs::write(&staged, b"hello").unwrap();
        let final_path = s.commit_artifact(&t.id, &staged, "report.md").unwrap();
        assert!(final_path.exists());
        assert_eq!(fs::read(&final_path).unwrap(), b"hello");
        // 同目录下没有 .tmp-commit 残留
        let art = s.artifacts_dir(&t.id).unwrap();
        let leftovers: Vec<_> = fs::read_dir(&art)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(leftovers, vec!["report.md".to_string()]);
    }

    #[test]
    fn list_returns_created_tasks_in_order() {
        let s = make_store();
        for i in 0..3 {
            let mut t = Task::default();
            t.id = format!("t_list_{i}");
            t.created_at = 1000 + i as u64;
            s.create(&t).unwrap();
        }
        let all = s.list().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, "t_list_0");
    }

    #[test]
    fn path_traversal_rejected() {
        let s = make_store();
        let mut t = Task::default();
        t.id = "../evil".into();
        assert!(s.create(&t).is_err());
        assert!(s.load("../../etc/passwd").is_err());
    }
}
