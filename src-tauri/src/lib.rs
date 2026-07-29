use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use dev_sweeper_core as core;

/// 当前扫描的取消标志。同一时刻只允许一个 scan 进行；新 scan 开始时重置。
/// 用 Arc 让 scan 线程（spawn_blocking）持有副本，cancel_scan 命令通过 State 置位。
type CancelSlot = Arc<Mutex<Option<Arc<AtomicBool>>>>;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SizeEvent {
    id: u32,
    size: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    scanned_dirs: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScanSummary {
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteReport {
    deleted: Vec<String>,
    failed: Vec<(String, String)>,
    /// dry_run=true 时 deleted 列表实际是"本会删除"的清单，未真正执行
    dry_run: bool,
}

#[tauri::command]
async fn scan(
    app: AppHandle,
    state: State<'_, CancelSlot>,
    root: String,
    rule_ids: Vec<String>,
    excludes: Vec<String>,
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
                let _ = app.emit("scan:found", a);
            },
            |scanned_dirs| {
                let _ = app_for_progress.emit("scan:progress", ProgressEvent { scanned_dirs });
            },
        );
        let app_for_size = app.clone();
        core::compute_sizes(&mut artifacts, &cancel, |id, size| {
            let _ = app_for_size.emit("scan:size", SizeEvent { id, size });
        });
        let cancelled = cancel.load(Ordering::Relaxed);
        // 扫描结束：清空 state 中的标志
        {
            let mut slot = cancel_slot.lock().map_err(|e| e.to_string())?;
            *slot = None;
        }
        let summary = ScanSummary {
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
    paths: Vec<String>,
    dry_run: bool,
) -> Result<DeleteReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let total = paths.len();
        let mut report = DeleteReport { deleted: Vec::new(), failed: Vec::new(), dry_run };
        for (i, path) in paths.into_iter().enumerate() {
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
                Ok(()) => report.deleted.push(path),
                Err(e) => report.failed.push((path, e)),
            }
        }
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage::<CancelSlot>(Arc::new(Mutex::new(None)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![scan, cancel_scan, delete_artifacts])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
