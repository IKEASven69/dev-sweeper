use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use dev_sweeper_core as core;
use dev_sweeper_core::{
    ArchiveFile, ArchiveReport, ArchivableProject, CacheEntry, CachePurgeReport, DepReport,
    MigratePyReport, MigrateReport, PruneReport, RestoreReport,
};

/// 当前扫描的取消标志。同一时刻只允许一个 scan 进行；新 scan 开始时重置。
/// 用 Arc 让 scan 线程（spawn_blocking）持有副本，cancel_scan 命令通过 State 置位。
type CancelSlot = Arc<Mutex<Option<Arc<AtomicBool>>>>;

/// 批量删除的取消槽：与扫描槽同构，但独立管理（Tauri State 按类型区分）。
/// 批量删除可能长达数分钟（每个目录都要走回收站），必须可中途取消。
#[derive(Clone)]
struct DeleteCancelSlot(CancelSlot);

/// 迁移（pnpm/uv 子进程）的取消槽：独立于扫描与删除，迁移子进程可被 kill。
#[derive(Clone)]
struct MigrateCancelSlot(CancelSlot);

/// migrate:log 事件载荷：迁移子进程的一行输出（stdout/stderr 逐行转发）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MigrateLogEvent {
    line: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FoundEvent {
    /// 扫描代际号：前端用它丢弃上一轮扫描的迟到事件（旧扫描被取消后其
    /// compute_sizes 仍可能发出按 id 碰撞的 scan:size，污染新扫描结果）
    gen: u32,
    artifact: dev_sweeper_core::Artifact,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SizeEvent {
    gen: u32,
    id: u32,
    size: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    gen: u32,
    scanned_dirs: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScanSummary {
    gen: u32,
    count: usize,
    total_bytes: u64,
    elapsed_ms: u64,
    /// 是否被 cancel_scan 中途取消
    cancelled: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeleteProgress {
    done: usize,
    total: usize,
    path: String,
    ok: bool,
    error: Option<String>,
}

/// 单个待删产物：路径 + 前端已知的元数据（决策日志要记 size 与陈旧天数，
/// 扫描阶段已经算过，删除时直接带上，避免重复统计 I/O）。
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeleteItem {
    path: String,
    size_bytes: Option<u64>,
    last_active_ms: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteReport {
    deleted: Vec<String>,
    failed: Vec<(String, String)>,
    /// dry_run=true 时 deleted 列表实际是"本会删除"的清单，未真正执行
    dry_run: bool,
    /// 被 cancel_delete 中途取消（剩余项未处理，保留在列表里）
    cancelled: bool,
}

#[tauri::command]
async fn scan(
    app: AppHandle,
    state: State<'_, CancelSlot>,
    root: String,
    rule_ids: Vec<String>,
    excludes: Vec<String>,
    gen: u32,
) -> Result<ScanSummary, String> {
    // State<'_> 不能跨 spawn_blocking（非 'static）；先 clone 出内部的 Arc。
    let cancel_slot: CancelSlot = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let rules = core::select_rules(&rule_ids);
        let start = Instant::now();
        // 为本次扫描创建取消标志并存入 state（供 cancel_scan 命令置位）
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
            *slot = Some(cancel.clone());
        }
        let app_for_progress = app.clone();
        let mut artifacts = core::scan_artifacts(
            Path::new(&root),
            &rules,
            &cancel,
            &excludes,
            |a| {
                let _ = app.emit("scan:found", FoundEvent { gen, artifact: a.clone() });
            },
            |scanned_dirs| {
                let _ = app_for_progress.emit(
                    "scan:progress",
                    ProgressEvent { gen, scanned_dirs },
                );
            },
        );
        let app_for_size = app.clone();
        core::compute_sizes(&mut artifacts, &cancel, |id, size| {
            let _ = app_for_size.emit("scan:size", SizeEvent { gen, id, size });
        });
        let cancelled = cancel.load(Ordering::Relaxed);
        // 扫描结束：清空 state 中的标志。仅当槽里仍是本轮注册的那个 Arc 时才清——
        // 若本轮收尾晚于新一轮扫描注册（极端竞态），无条件清空会误清新一轮的
        // 取消标志，导致新一轮扫描无法取消。
        {
            let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
            if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &cancel)) {
                *slot = None;
            }
        }
        let summary = ScanSummary {
            gen,
            count: artifacts.len(),
            total_bytes: artifacts.iter().filter_map(|a| a.size_bytes).sum(),
            elapsed_ms: start.elapsed().as_millis() as u64,
            cancelled,
        };
        let _ = app.emit("scan:done", &summary);
        Ok(summary)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 取消正在进行的扫描（若有）。立即返回。
#[tauri::command]
async fn cancel_scan(state: State<'_, CancelSlot>) -> Result<bool, String> {
    let slot = state.lock().map_err(|e| e.to_string())?;
    if let Some(cancel) = slot.as_ref() {
        cancel.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        Ok(false) // 没有正在进行的扫描
    }
}

#[tauri::command]
async fn delete_artifacts(
    app: AppHandle,
    state: State<'_, DeleteCancelSlot>,
    items: Vec<DeleteItem>,
    dry_run: bool,
    log_decisions: bool,
) -> Result<DeleteReport, String> {
    // State<'_> 不能跨 spawn_blocking；先 clone 出内部 Arc。
    let cancel_slot: CancelSlot = state.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let total = items.len();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut report = DeleteReport {
            deleted: Vec::new(),
            failed: Vec::new(),
            dry_run,
            cancelled: false,
        };
        // 注册本轮删除的取消标志（供 cancel_delete 置位）
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
            *slot = Some(cancel.clone());
        }
        for (i, item) in items.into_iter().enumerate() {
            // 取消检查点：每个条目处理前检查，置位即停（剩余项不处理也不算失败）
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            let path = item.path;
            let result = if dry_run {
                core::delete_to_trash_dry_run(Path::new(&path))
            } else {
                core::delete_to_trash(Path::new(&path))
            };
            let _ = app.emit(
                "delete:progress",
                DeleteProgress {
                    done: i + 1,
                    total,
                    path: path.clone(),
                    ok: result.is_ok(),
                    error: result.as_ref().err().cloned(),
                },
            );
            match result {
                Ok(()) => {
                    // 决策日志（opt-in）：只记真实删除，预演不是决策
                    if log_decisions && !dry_run {
                        let stale_days = item
                            .last_active_ms
                            .map(|t| now_ms.saturating_sub(t) / 86_400_000);
                        if let Err(e) =
                            core::append_decision(
                                true,
                                &core::Decision::delete(&path, item.size_bytes, stale_days),
                            )
                        {
                            eprintln!("warning: 决策日志写入失败: {e}");
                        }
                    }
                    report.deleted.push(path);
                }
                Err(e) => report.failed.push((path, e)),
            }
        }
        // 收尾：仅当槽里仍是本轮注册的 Arc 时才清（与 scan 收尾同一竞态防护）
        {
            let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
            if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &cancel)) {
                *slot = None;
            }
        }
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 取消正在进行的批量删除（若有）。已在回收站里的项不会恢复。立即返回。
#[tauri::command]
async fn cancel_delete(state: State<'_, DeleteCancelSlot>) -> Result<bool, String> {
    let slot = state.inner().0.lock().map_err(|e| e.to_string())?;
    if let Some(cancel) = slot.as_ref() {
        cancel.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        Ok(false) // 没有正在进行的删除
    }
}

#[tauri::command]
async fn analyze_deps(project_dir: String) -> Result<DepReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        dev_sweeper_core::analyze_deps(Path::new(&project_dir))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn prune_deps(
    project_dir: String,
    remove: Vec<String>,
    dry_run: bool,
    log_decisions: bool,
) -> Result<PruneReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let rep = dev_sweeper_core::prune_deps(Path::new(&project_dir), &remove, dry_run)?;
        // 决策日志（opt-in）：只记真实裁剪
        if log_decisions && !dry_run {
            for name in &rep.removed {
                if let Err(e) =
                    dev_sweeper_core::append_decision(true, &dev_sweeper_core::Decision::prune(name, None))
                {
                    eprintln!("warning: 决策日志写入失败: {e}");
                }
            }
        }
        Ok(rep)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn migrate_to_pnpm(
    app: AppHandle,
    state: State<'_, MigrateCancelSlot>,
    project_dir: String,
    dry_run: bool,
) -> Result<MigrateReport, String> {
    // State<'_> 不能跨 spawn_blocking；先 clone 出内部 Arc。
    let cancel_slot: CancelSlot = state.inner().0.clone();
    // 注册本轮迁移的取消标志（供 cancel_migrate 置位）
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
        *slot = Some(cancel.clone());
    }
    let app_for_log = app.clone();
    let cancel_for_task = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // 子进程每行输出转成事件发给前端；core 层本身不发事件
        dev_sweeper_core::migrate_to_pnpm(Path::new(&project_dir), dry_run, &cancel_for_task, |line| {
            let _ = app_for_log.emit("migrate:log", MigrateLogEvent { line: line.to_string() });
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    // 收尾：仅当槽里仍是本轮注册的 Arc 时才清（与 scan 收尾同一竞态防护）
    {
        let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
        if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &cancel)) {
            *slot = None;
        }
    }
    result
}

#[tauri::command]
async fn migrate_to_uv(
    app: AppHandle,
    state: State<'_, MigrateCancelSlot>,
    project_dir: String,
    dry_run: bool,
) -> Result<MigratePyReport, String> {
    let cancel_slot: CancelSlot = state.inner().0.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
        *slot = Some(cancel.clone());
    }
    let app_for_log = app.clone();
    let cancel_for_task = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        dev_sweeper_core::migrate_to_uv(Path::new(&project_dir), dry_run, &cancel_for_task, |line| {
            let _ = app_for_log.emit("migrate:log", MigrateLogEvent { line: line.to_string() });
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    {
        let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
        if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &cancel)) {
            *slot = None;
        }
    }
    result
}

/// 取消正在进行的迁移（若有）：kill 迁移子进程，已完成步骤不回滚。立即返回。
#[tauri::command]
async fn cancel_migrate(state: State<'_, MigrateCancelSlot>) -> Result<bool, String> {
    let slot = state.inner().0.lock().map_err(|e| e.to_string())?;
    if let Some(cancel) = slot.as_ref() {
        cancel.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        Ok(false) // 没有正在进行的迁移
    }
}

#[tauri::command]
async fn discover_caches() -> Result<Vec<CacheEntry>, String> {
    let caches = tauri::async_runtime::spawn_blocking(dev_sweeper_core::discover_global_caches)
        .await
        .map_err(|e| e.to_string())?;
    Ok(caches)
}

#[tauri::command]
async fn purge_cache(id: String, dry_run: bool) -> Result<CachePurgeReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        dev_sweeper_core::purge_cache(&id, dry_run)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn discover_archivable(
    root: String,
    stale_days: u64,
    excludes: Vec<String>,
) -> Result<Vec<ArchivableProject>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(dev_sweeper_core::discover_archivable(Path::new(&root), stale_days, &excludes))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn archive_project(
    dir: String,
    archive_dir: String,
    dry_run: bool,
    excludes: Vec<String>,
) -> Result<ArchiveReport, String> {
    let adir = if archive_dir.is_empty() {
        dev_sweeper_core::default_archive_dir()
    } else {
        archive_dir
    };
    tauri::async_runtime::spawn_blocking(move || {
        dev_sweeper_core::archive_project(Path::new(&dir), Path::new(&adir), dry_run, &excludes)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_archives(archive_dir: String) -> Result<Vec<ArchiveFile>, String> {
    let adir = if archive_dir.is_empty() {
        dev_sweeper_core::default_archive_dir()
    } else {
        archive_dir
    };
    tauri::async_runtime::spawn_blocking(move || {
        dev_sweeper_core::list_archives(Path::new(&adir))
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn restore_archive(
    file: String,
    dest_root: String,
    dry_run: bool,
) -> Result<RestoreReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        dev_sweeper_core::restore_archive(Path::new(&file), Path::new(&dest_root), dry_run)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage::<CancelSlot>(Arc::new(Mutex::new(None)))
        .manage::<DeleteCancelSlot>(DeleteCancelSlot(Arc::new(Mutex::new(None))))
        .manage::<MigrateCancelSlot>(MigrateCancelSlot(Arc::new(Mutex::new(None))))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            scan,
            cancel_scan,
            delete_artifacts,
            cancel_delete,
            analyze_deps,
            prune_deps,
            migrate_to_pnpm,
            migrate_to_uv,
            cancel_migrate,
            discover_caches,
            purge_cache,
            discover_archivable,
            archive_project,
            list_archives,
            restore_archive
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
