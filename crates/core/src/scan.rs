use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use serde::Serialize;
use walkdir::WalkDir;

use crate::rules::{dir_name_matches, marker_ok, CleanRule};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: u32,
    pub rule_id: String,
    pub path: String,
    pub project_dir: String,
    pub project_name: String,
    /// None = 尚未计算
    pub size_bytes: Option<u64>,
    /// 项目最后活跃时间（epoch 毫秒），None = 无法确定
    pub last_active_ms: Option<u64>,
    pub regen_hint: String,
}

/// 单遍遍历：目录名命中规则且标记确认 → 记为 Artifact 并不再深入；跳过 .git；
/// 不 follow symlink；遍历错误（如无权限）静默跳过。
///
/// - `cancel`：置 true 则尽快终止（下次循环检查点退出），返回已发现的部分结果。
/// - `on_found`：每发现一个产物即回调（流式 UI 用）。
/// - `on_progress`：每扫约 256 个目录回调一次已扫目录数（进度条用）。
pub fn scan_artifacts(
    root: &Path,
    rules: &[&'static CleanRule],
    cancel: &AtomicBool,
    mut on_found: impl FnMut(&Artifact),
    mut on_progress: impl FnMut(usize),
) -> Vec<Artifact> {
    let mut found = Vec::new();
    let mut scanned_dirs = 0usize;
    let mut it = WalkDir::new(root).follow_links(false).into_iter();
    while let Some(entry) = it.next() {
        // 取消检查点：在每次取下一个条目前检查
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            continue;
        }
        scanned_dirs = scanned_dirs.wrapping_add(1);
        // 每 256 个目录回报一次进度，避免 emit 过密
        if scanned_dirs & 0xFF == 0 {
            on_progress(scanned_dirs);
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            it.skip_current_dir();
            continue;
        }
        if let Some(rule) = match_rule(entry.path(), &name, rules) {
            it.skip_current_dir();
            let artifact = build_artifact(found.len() as u32, rule, entry.path());
            on_found(&artifact);
            found.push(artifact);
        }
    }
    // 终态进度（无论取消与否，让 UI 收尾）
    on_progress(scanned_dirs);
    found
}

fn match_rule(path: &Path, name: &str, rules: &[&'static CleanRule]) -> Option<&'static CleanRule> {
    rules
        .iter()
        .find(|rule| dir_name_matches(rule, name) && marker_ok(rule, path))
        .copied()
}

fn build_artifact(id: u32, rule: &'static CleanRule, path: &Path) -> Artifact {
    let project_dir = path.parent().unwrap_or(path);
    Artifact {
        id,
        rule_id: rule.id.to_string(),
        path: path.to_string_lossy().into_owned(),
        project_dir: project_dir.to_string_lossy().into_owned(),
        project_name: project_name(rule, project_dir),
        size_bytes: None,
        last_active_ms: last_active_ms(project_dir),
        regen_hint: rule.regen_hint.to_string(),
    }
}

fn project_name(rule: &CleanRule, project_dir: &Path) -> String {
    if rule.id == "node" {
        if let Ok(text) = std::fs::read_to_string(project_dir.join("package.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
                    return name.to_string();
                }
            }
        }
    }
    project_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| project_dir.to_string_lossy().into_owned())
}

/// 项目最后活跃时间：取各生态标记文件与 src/ 目录 mtime 的最大值。
fn last_active_ms(project_dir: &Path) -> Option<u64> {
    const CANDIDATES: &[&str] = &[
        "package.json",
        "Cargo.toml",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "pyproject.toml",
        "requirements.txt",
        "src",
    ];
    CANDIDATES
        .iter()
        .filter_map(|c| std::fs::metadata(project_dir.join(c)).ok())
        .chain(std::fs::metadata(project_dir).ok())
        .filter_map(|m| m.modified().ok())
        .filter_map(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .max()
}

/// rayon 并行计算每个产物目录的大小，算完一个回调一个。
///
/// `cancel` 置 true 后，尚未开始的任务会被跳过（已开始的会算完）。
/// 大小未算出的产物其 `size_bytes` 仍为 `None`。
pub fn compute_sizes(
    artifacts: &mut [Artifact],
    cancel: &AtomicBool,
    on_sized: impl Fn(u32, u64) + Sync,
) {
    artifacts.par_iter_mut().for_each(|a| {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let size = dir_size(Path::new(&a.path));
        a.size_bytes = Some(size);
        on_sized(a.id, size);
    });
}

pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}
