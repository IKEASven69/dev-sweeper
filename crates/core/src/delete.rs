use std::path::Path;

use crate::rules::known_dir_names;

/// 删除前校验：路径末段必须是已知产物目录名，防止误删任意目录。
pub fn validate_artifact_path(path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("无效路径: {}", path.display()))?;
    if known_dir_names().contains(&name.as_str()) {
        Ok(())
    } else {
        Err(format!("拒绝删除非产物目录: {}", path.display()))
    }
}

/// 移入回收站（不做永久删除）。
pub fn delete_to_trash(path: &Path) -> Result<(), String> {
    validate_artifact_path(path)?;
    if !path.exists() {
        return Err(format!("路径不存在: {}", path.display()));
    }
    trash::delete(path).map_err(|e| format!("移入回收站失败: {e}"))
}
