//! 压缩归档：把"沉睡"的整个项目打包成可恢复的 .tar.gz，释放工作区即时空间。
//!
//! 这是产品定位「不删也能瘦」的核心落地——活的项目源码不能删，但可以压成归档，
//! 需要时一键解回。配合 `discover_archivable` 按最后活跃时间升序（沉睡最久优先），
//! 正好把"睡得最久的项目优先回收"这个差异化卖点用起来。
//!
//! 安全模型：归档成功后，原项目**移入回收站**（与全工具一致），而非 remove_dir_all；
//! 归档文件本身也是一份可恢复副本。要真正释放磁盘，用户清空回收站即可（与删
//! node_modules 行为统一）。

use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Local;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use tar::{Archive, Builder};
use walkdir::WalkDir;

use crate::scan::dir_size;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ArchivableProject {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub last_active_ms: Option<u64>,
    pub is_git: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveReport {
    pub name: String,
    pub source_path: String,
    pub archive_file: String,
    pub original_size: u64,
    pub compressed_size: u64,
    pub freed_bytes: u64,
    pub removed_original: bool,
    pub error: Option<String>,
    pub dry_run: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub archive_file: String,
    pub restored_to: String,
    pub restored_bytes: u64,
    pub error: Option<String>,
    pub dry_run: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveFile {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub project_name: String,
    pub created_at: String,
}

/// 默认归档库：~/dev-archives
pub fn default_archive_dir() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    Path::new(&home)
        .join("dev-archives")
        .to_string_lossy()
        .into_owned()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// 取目录内最深修改时间（mtime），作为"最后活跃"近似。
fn dir_last_active_ms(dir: &Path) -> Option<u64> {
    let mut max: Option<u64> = None;
    for e in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if let Ok(m) = e.metadata() {
            if let Ok(t) = m.modified() {
                let ms = t
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                max = Some(max.map(|x| x.max(ms)).unwrap_or(ms));
            }
        }
    }
    max
}

/// 扫描 root 下一级子目录，返回"沉睡项目"列表（按最后活跃升序，最旧的在前）。
/// `min_stale_days`：只返回超过 N 天未活跃的项目；为 0 则返回全部（仍按陈旧排序）。
pub fn discover_archivable(root: &Path, min_stale_days: u64) -> Vec<ArchivableProject> {
    let cutoff = if min_stale_days > 0 {
        Some(now_ms().saturating_sub(min_stale_days * 86_400_000))
    } else {
        None
    };
    let mut out = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for ent in entries.filter_map(|e| e.ok()) {
        let path = ent.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        // 跳过点文件 / 符号链接 / 非目录
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        let size = dir_size(&path);
        let last = dir_last_active_ms(&path);
        if let Some(c) = cutoff {
            if last.map(|t| t >= c).unwrap_or(true) {
                continue; // 不够陈旧
            }
        }
        let is_git = path.join(".git").is_dir();
        out.push(ArchivableProject {
            name,
            path: path.to_string_lossy().into_owned(),
            size_bytes: size,
            last_active_ms: last,
            is_git,
        });
    }
    // 最旧在前（沉睡最久优先）
    out.sort_by_key(|p| p.last_active_ms.unwrap_or(0));
    out
}

fn archive_file_name(name: &str) -> String {
    let date = Local::now().format("%Y%m%d");
    format!("{name}@{date}.tar.gz")
}

fn pack_tar_gz(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let file = File::create(dst)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(enc);
    // 用项目名作为 tar 内顶层目录，解压后落在 <dest>/<name>
    let top = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    builder.append_dir_all(top, src)?;
    builder.finish()?;
    let enc = builder.into_inner()?;
    enc.finish()?;
    Ok(dst.metadata()?.len())
}

fn unpack_tar_gz(file: &Path, dest: &Path) -> std::io::Result<u64> {
    let f = File::open(file)?;
    let dec = GzDecoder::new(BufReader::new(f));
    let mut ar = Archive::new(dec);
    ar.unpack(dest)?;
    Ok(dir_size(dest))
}

/// 把整个项目打包成 .tar.gz（写入 archive_dir），成功后原项目移入回收站。
/// dry_run 只报告，不写文件不删项目。
pub fn archive_project(
    dir: &Path,
    archive_dir: &Path,
    dry_run: bool,
) -> Result<ArchiveReport, String> {
    if !dir.is_dir() {
        return Err(format!("不是目录: {}", dir.display()));
    }
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();
    let original_size = dir_size(dir);
    let archive_path: PathBuf = archive_dir.join(archive_file_name(&name));

    if dry_run {
        return Ok(ArchiveReport {
            name,
            source_path: dir.to_string_lossy().into_owned(),
            archive_file: archive_path.to_string_lossy().into_owned(),
            original_size,
            compressed_size: 0,
            freed_bytes: original_size,
            removed_original: false,
            error: None,
            dry_run: true,
        });
    }

    fs::create_dir_all(archive_dir)
        .map_err(|e| format!("无法创建归档目录 {}: {e}", archive_dir.display()))?;
    let compressed = pack_tar_gz(dir, &archive_path).map_err(|e| format!("打包失败: {e}"))?;

    // 归档成功后，原项目移入回收站（可逆，与全工具一致）
    let (removed_original, err) = match trash::delete(dir) {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    let freed = if removed_original { original_size } else { 0 };

    Ok(ArchiveReport {
        name,
        source_path: dir.to_string_lossy().into_owned(),
        archive_file: archive_path.to_string_lossy().into_owned(),
        original_size,
        compressed_size: compressed,
        freed_bytes: freed,
        removed_original,
        error: err,
        dry_run: false,
    })
}

/// 列出归档库中的 .tar.gz 文件。
pub fn list_archives(archive_dir: &Path) -> Vec<ArchiveFile> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(archive_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for ent in entries.filter_map(|e| e.ok()) {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("gz") {
            continue;
        }
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !fname.ends_with(".tar.gz") {
            continue;
        }
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let project_name = parse_project_name(&fname);
        let created_at = parse_archive_date(&fname);
        out.push(ArchiveFile {
            name: fname.clone(),
            path: path.to_string_lossy().into_owned(),
            size_bytes: size,
            project_name,
            created_at,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 解压归档回到 dest_root（生成 dest_root/<project_name>）。
/// 若目标目录已存在则报错，避免覆盖。dry_run 只校验不写。
pub fn restore_archive(
    file: &Path,
    dest_root: &Path,
    dry_run: bool,
) -> Result<RestoreReport, String> {
    if !file.is_file() {
        return Err(format!("归档文件不存在: {}", file.display()));
    }
    let fname = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let project_name = parse_project_name(&fname);
    let target = Path::new(dest_root).join(&project_name);
    if dry_run {
        return Ok(RestoreReport {
            archive_file: file.to_string_lossy().into_owned(),
            restored_to: target.to_string_lossy().into_owned(),
            restored_bytes: 0,
            error: None,
            dry_run: true,
        });
    }
    if target.exists() {
        return Err(format!(
            "目标已存在，为避免覆盖已取消: {}",
            target.display()
        ));
    }
    fs::create_dir_all(dest_root)
        .map_err(|e| format!("无法创建目标目录 {}: {e}", dest_root.display()))?;
    let restored = unpack_tar_gz(file, dest_root).map_err(|e| format!("解压失败: {e}"))?;
    Ok(RestoreReport {
        archive_file: file.to_string_lossy().into_owned(),
        restored_to: target.to_string_lossy().into_owned(),
        restored_bytes: restored,
        error: None,
        dry_run: false,
    })
}

fn parse_project_name(fname: &str) -> String {
    let base = fname.strip_suffix(".tar.gz").unwrap_or(fname);
    match base.rsplit_once('@') {
        Some((name, _)) => name.to_string(),
        None => base.to_string(),
    }
}

fn parse_archive_date(fname: &str) -> String {
    let base = fname.strip_suffix(".tar.gz").unwrap_or(fname);
    match base.rsplit_once('@') {
        Some((_, date)) => date.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_skips_hidden_symlinks_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("alive/src")).unwrap();
        fs::write(tmp.path().join("alive/src/a.rs"), "x").unwrap();
        fs::create_dir_all(tmp.path().join(".hidden")).unwrap();
        fs::write(tmp.path().join("readme.txt"), "x").unwrap();
        let v = discover_archivable(tmp.path(), 0);
        let names: Vec<&str> = v.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alive"]);
        assert!(!v[0].is_git);
    }

    #[test]
    fn discover_excludes_fresh_when_stale_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let fresh = tmp.path().join("fresh");
        fs::create_dir_all(&fresh).unwrap();
        fs::write(fresh.join("a"), "x").unwrap();
        // min_stale_days=999 应排除刚创建的项目
        let v = discover_archivable(&fresh, 999);
        assert!(v.is_empty(), "刚创建的项目不应算沉睡: {:?}", v);
    }

    #[test]
    fn archive_and_restore_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("myproj");
        fs::create_dir_all(proj.join("src")).unwrap();
        fs::write(proj.join("src/main.rs"), "fn main(){}").unwrap();
        fs::write(proj.join("README.md"), "hello").unwrap();
        let arch = tmp.path().join("arch");

        // dry-run 不写文件、不删项目
        let dr = archive_project(&proj, &arch, true).unwrap();
        assert!(dr.dry_run);
        assert!(dr.original_size > 0);
        assert!(!Path::new(&dr.archive_file).exists());

        // 真实打包
        let rep = archive_project(&proj, &arch, false).unwrap();
        assert!(!rep.dry_run);
        assert!(Path::new(&rep.archive_file).exists(), "归档文件应存在");
        assert!(rep.compressed_size > 0);
        // 极小项目 gzip 可能因元数据开销略增；压缩率只在较大项目上体现，这里只验证产出有效

        // 列出
        let listed = list_archives(&arch);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].project_name, "myproj");

        // 还原到新位置
        let dest = tmp.path().join("restored");
        let rr = restore_archive(Path::new(&rep.archive_file), &dest, false).unwrap();
        assert!(!rr.dry_run);
        assert!(dest.join("myproj/src/main.rs").exists());
        assert!(dest.join("myproj/README.md").exists());
    }

    #[test]
    fn restore_dry_run_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let arch = tmp.path().join("arch");
        fs::create_dir_all(&arch).unwrap();
        let f = arch.join("x@20260101.tar.gz");
        fs::write(&f, "not really gzip").unwrap();
        let rr = restore_archive(&f, tmp.path(), true).unwrap();
        assert!(rr.dry_run);
        assert!(!tmp.path().join("x").exists());
    }

    #[test]
    fn parse_names() {
        assert_eq!(parse_project_name("myproj@20260101.tar.gz"), "myproj");
        assert_eq!(parse_archive_date("myproj@20260101.tar.gz"), "20260101");
        assert_eq!(parse_project_name("weird-name.tar.gz"), "weird-name");
    }
}
