//! 删除模块——**安全不变量**（见 DESIGN.md §5）：
//!
//! 1. 任何删除路径必经 `validate_marker`（名 + marker 双闸）。
//! 2. 删除一律走 `trash::delete`（移入回收站），**永不** `std::fs::remove_dir_all`。
//!    这是工具的头号卖点与安全底线，不提供"永久删除"开关，不可关闭。
//! 3. 提供 `delete_to_trash_dry_run` 供预演：校验通过但不执行。

use std::path::Path;

use crate::rules::validate_marker;

/// 删除前校验：路径末段必须是已知产物目录名 **且** 该规则的 marker 仍成立。
///
/// 双闸设计——既防"删非产物目录"，也防"删同名但无 marker 的目录"。
/// 走与扫描时相同的 `validate_marker` 判定，保证删除校验自洽。
pub fn validate_artifact_path(path: &Path) -> Result<(), String> {
    validate_marker(path).map(|_| ())
}

/// 移入回收站（**不做永久删除**——这是工具的头号安全卖点，不可关闭）。
///
/// `dry_run = true` 时只做校验和存在性检查，不真正执行 trash::delete，
/// 返回的路径表示"本会移入回收站"。
pub fn delete_to_trash(path: &Path) -> Result<(), String> {
    delete_to_trash_with(path, false)
}

/// 预演：只校验、不执行。返回"本会移入回收站"的路径（即入参）。
pub fn delete_to_trash_dry_run(path: &Path) -> Result<(), String> {
    delete_to_trash_with(path, true)
}

fn delete_to_trash_with(path: &Path, dry_run: bool) -> Result<(), String> {
    validate_marker(path)?;
    if !path.exists() {
        return Err(format!("路径不存在: {}", path.display()));
    }
    if dry_run {
        return Ok(());
    }
    // 一律 trash，永不 remove_dir_all——这是工具的安全底线（见 DESIGN.md §5）。
    trash::delete(path).map_err(|e| format!("移入回收站失败: {e}"))
}
