//! 决策日志（opt-in）：把删除/裁剪决策以 JSONL 追加到 `~/.dev-sweeper/decisions.jsonl`。
//!
//! 目的：事后可审计"当时删了什么、多大、多陈旧"——清理工具的决策可回溯
//! 比不可回溯更让人放心。默认**关闭**：不主动留痕也是隐私诉求，由
//! CLI `--log-decisions` / GUI 设置开关显式开启。
//!
//! 写失败只让调用方告警，不阻断清理主流程；文件按行追加（JSONL），
//! 多条决策各自独立成行，损坏一行不影响其余。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// 决策类型。
pub const KIND_DELETE: &str = "delete";
/// 裁剪依赖决策类型。
pub const KIND_PRUNE: &str = "prune";

/// 一条决策记录（JSONL 中的一行）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    /// 决策时间（unix 毫秒）
    pub ts: u64,
    /// 决策类型：delete（删产物）/ prune（裁剪依赖）
    pub kind: &'static str,
    /// 产物路径（delete）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 依赖名（prune）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dep: Option<String>,
    /// 涉及体积（字节；调用方已知才填）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// 决策时的陈旧天数（last_active 距今；调用方已知才填）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_days: Option<u64>,
}

impl Decision {
    /// 删除产物决策。
    pub fn delete(path: &str, size: Option<u64>, stale_days: Option<u64>) -> Self {
        Self::now(KIND_DELETE, Some(path.to_string()), None, size, stale_days)
    }

    /// 裁剪依赖决策（依赖名 + 可知的释放体积）。
    pub fn prune(dep: &str, size: Option<u64>) -> Self {
        Self::now(KIND_PRUNE, None, Some(dep.to_string()), size, None)
    }

    fn now(
        kind: &'static str,
        path: Option<String>,
        dep: Option<String>,
        size: Option<u64>,
        stale_days: Option<u64>,
    ) -> Self {
        Self {
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            kind,
            path,
            dep,
            size,
            stale_days,
        }
    }
}

/// 决策日志文件路径：`~/.dev-sweeper/decisions.jsonl`（home 不可得时 None）。
pub fn decisions_file() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".dev-sweeper").join("decisions.jsonl"))
}

/// 追加一条决策。`enabled = false` 时是 no-op（opt-in 开关由调用方持有并
/// 原样传入）。写失败返回 Err，调用方告警即可、不应阻断清理。
pub fn append_decision(enabled: bool, d: &Decision) -> std::io::Result<()> {
    if !enabled {
        return Ok(());
    }
    let file = decisions_file().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "无法定位用户主目录")
    })?;
    append_decision_to(&file, d)
}

/// 追加到指定文件（测试用注入路径的入口）。
fn append_decision_to(file: &Path, d: &Decision) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(d)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    f.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_jsonl_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("nested").join("decisions.jsonl");
        let d1 = Decision::delete("D:/a/node_modules", Some(1024), Some(30));
        append_decision_to(&file, &d1).unwrap();
        let d2 = Decision::prune("left-pad", None);
        append_decision_to(&file, &d2).unwrap();

        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "两条决策各自一行: {text}");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["kind"], "delete");
        assert_eq!(first["path"], "D:/a/node_modules");
        assert_eq!(first["size"], 1024);
        assert_eq!(first["staleDays"], 30);
        assert!(first.get("dep").is_none(), "prune 专属字段在 delete 行应省略");
        assert!(first["ts"].as_u64().unwrap() > 0);
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["kind"], "prune");
        assert_eq!(second["dep"], "left-pad");
        assert!(second.get("path").is_none());
    }

    #[test]
    fn disabled_is_noop() {
        // enabled=false 直接返回 Ok，不解析路径、不落盘
        assert!(append_decision(false, &Decision::delete("x", None, None)).is_ok());
    }
}
