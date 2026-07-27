use std::path::Path;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use dev_sweeper_core as core;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SizeEvent {
    id: u32,
    size: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScanSummary {
    count: usize,
    total_bytes: u64,
    elapsed_ms: u64,
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
}

#[tauri::command]
async fn scan(app: AppHandle, root: String, rule_ids: Vec<String>) -> Result<ScanSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let rules = core::select_rules(&rule_ids);
        let start = Instant::now();
        let mut artifacts = core::scan_artifacts(Path::new(&root), &rules, |a| {
            let _ = app.emit("scan:found", a);
        });
        core::compute_sizes(&mut artifacts, |id, size| {
            let _ = app.emit("scan:size", SizeEvent { id, size });
        });
        let summary = ScanSummary {
            count: artifacts.len(),
            total_bytes: artifacts.iter().filter_map(|a| a.size_bytes).sum(),
            elapsed_ms: start.elapsed().as_millis() as u64,
        };
        let _ = app.emit("scan:done", &summary);
        Ok(summary)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_artifacts(app: AppHandle, paths: Vec<String>) -> Result<DeleteReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let total = paths.len();
        let mut report = DeleteReport { deleted: Vec::new(), failed: Vec::new() };
        for (i, path) in paths.into_iter().enumerate() {
            let result = core::delete_to_trash(Path::new(&path));
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![scan, delete_artifacts])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
