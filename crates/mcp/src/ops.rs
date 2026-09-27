//! MCP 工具的纯逻辑层：参数结构、双开关守卫、core 调用编排。
//! 刻意不依赖 rmcp——参数解析（serde）与守卫可以脱离协议层单测。

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use dev_sweeper_core::{
    analyze_deps, compute_sizes, delete_to_trash, delete_to_trash_dry_run,
    discover_archivable, discover_global_caches, purge_cache, scan_artifacts, select_rules,
    Artifact,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// 永不取消的标志。MCP stdio 会话没有取消语义（客户端断开即进程退出），
/// 与 CLI 相同：提供永不置位的标志以统一 core 的调用签名。
pub fn never_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

/// scan / clean 共用的扫描参数。
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanArgs {
    /// 扫描根目录（建议绝对路径）
    pub root: String,
    /// 规则 id 过滤（node/rust/maven/gradle/python-venv/python-cache/web-dist/cmake/dotnet/unity/unreal/cocoapods/composer 等），省略 = 全部
    pub rules: Option<Vec<String>>,
    /// 只保留超过 N 天未活跃的产物；省略 = 不过滤
    pub stale_days: Option<u64>,
    /// 排除（保护）路径前缀：命中的产物不扫描、不删除
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// clean 参数。真实执行必须 dry_run=false 且 confirm=true 双开关同时满足。
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanArgs {
    /// 扫描根目录（建议绝对路径）
    pub root: String,
    /// 规则 id 过滤，省略 = 全部
    pub rules: Option<Vec<String>>,
    /// 只清理超过 N 天未活跃的产物；省略 = 不过滤
    pub stale_days: Option<u64>,
    /// 排除（保护）路径前缀：命中的产物不扫描、不删除
    #[serde(default)]
    pub exclude: Vec<String>,
    /// 预演开关，默认 true（只校验与报告，不动回收站）
    pub dry_run: Option<bool>,
    /// 真实执行确认。dry_run=false 时必须显式传 confirm=true，否则拒绝
    #[serde(default)]
    pub confirm: bool,
}

/// deps 参数。
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DepsArgs {
    /// 项目目录（须含 package.json）
    pub project_dir: String,
}

/// caches_purge 参数。真实执行同样要求 dry_run=false 且 confirm=true。
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PurgeArgs {
    /// 要清理的缓存 id 列表（来自 caches 工具的返回）
    #[serde(default)]
    pub ids: Vec<String>,
    /// 预演开关，默认 true
    pub dry_run: Option<bool>,
    /// 真实执行确认
    #[serde(default)]
    pub confirm: bool,
}

/// archives_discover 参数。
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchivesArgs {
    /// 根目录（扫描其下一级子目录）
    pub root: String,
    /// 只返回超过 N 天未活跃的项目；省略 = 全部（仍按陈旧升序）
    pub stale_days: Option<u64>,
    /// 排除（保护）路径前缀：命中的项目不列出
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// 双开关守卫的裁决结果。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ExecMode {
    DryRun,
    Real,
}

/// 双开关守卫：真实破坏性操作必须同时给出 `dry_run=false` 与 `confirm=true`。
///
/// - `dry_run` 缺省按 true 处理（安全默认：不传参数只可能预演）；
/// - 只要 dry_run 为 true，confirm 传什么都不进入真实执行；
/// - dry_run=false 而 confirm 缺省/false → 拒绝并说明原因（调用方把该消息
///   原样返回给 agent）。
pub fn resolve_mode(tool_name: &str, dry_run: Option<bool>, confirm: bool) -> Result<ExecMode, String> {
    let dry = dry_run.unwrap_or(true);
    if !dry && !confirm {
        return Err(format!(
            "已拒绝执行 {tool_name}：真实执行需要 dry_run=false 且 confirm=true 双开关同时满足，\
             当前缺少 confirm=true。未做任何修改。建议先以 dry_run=true（默认）预演，\
             确认清单后再同时传 dry_run=false 与 confirm=true。"
        ));
    }
    Ok(if dry { ExecMode::DryRun } else { ExecMode::Real })
}

/// 校验规则 id 全部已知；未知即报错并列出全部合法 id。
/// MCP 调用方是 agent，拼错规则名应该立刻失败而不是静默忽略。
fn check_rules(rule_ids: &[String]) -> Result<(), String> {
    let known: std::collections::HashSet<&str> = dev_sweeper_core::RULES.iter().map(|r| r.id).collect();
    let bad: Vec<&String> = rule_ids.iter().filter(|id| !known.contains(id.as_str())).collect();
    if bad.is_empty() {
        return Ok(());
    }
    let all: Vec<&str> = dev_sweeper_core::RULES.iter().map(|r| r.id).collect();
    Err(format!(
        "未知规则 id：{}。合法 id：{}",
        bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
        all.join(",")
    ))
}

/// stale_days → epoch 毫秒截止线。core 的过滤口径与 CLI 一致：
/// last_active_ms 早于截止线的产物才保留。
fn stale_cutoff_ms(stale_days: Option<u64>) -> Option<u64> {
    stale_days.map(|days| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        // days * 86_400_000 在极大天数时会乘法溢出，饱和乘即可
        now.saturating_sub(days.saturating_mul(24 * 3600 * 1000))
    })
}

/// 扫描并计算大小（与 CLI `sweep scan` 同口径：过滤规则、排除前缀、stale 过滤、
/// 按体积降序）。stale_days 过滤沿用 CLI 语义：只保留"最后活跃早于截止线"的产物。
pub fn scan_and_size(args: &ScanArgs) -> Result<Vec<Artifact>, String> {
    let rule_ids = args.rules.clone().unwrap_or_default();
    check_rules(&rule_ids)?;
    let rules = select_rules(&rule_ids);
    let cancel = never_cancel();
    let mut artifacts =
        scan_artifacts(Path::new(&args.root), &rules, &cancel, &args.exclude, |_| {}, |_| {});
    compute_sizes(&mut artifacts, &cancel, |_, _| {});
    if let Some(cutoff) = stale_cutoff_ms(args.stale_days) {
        artifacts.retain(|a| a.last_active_ms.is_some_and(|t| t < cutoff));
    }
    artifacts.sort_by_key(|a| std::cmp::Reverse(a.size_bytes.unwrap_or(0)));
    Ok(artifacts)
}

/// clean：预演（marker/symlink 校验 + 统计）或真实移入回收站。
/// 返回结构化报告 JSON；守卫失败返回 Err（消息直接给 agent 看）。
pub fn clean(args: &CleanArgs) -> Result<serde_json::Value, String> {
    let mode = resolve_mode("clean", args.dry_run, args.confirm)?;
    let artifacts = scan_and_size(&ScanArgs {
        root: args.root.clone(),
        rules: args.rules.clone(),
        stale_days: args.stale_days,
        exclude: args.exclude.clone(),
    })?;

    match mode {
        ExecMode::DryRun => {
            let mut would_free = 0u64;
            let mut would_delete = 0usize;
            let mut rejected = Vec::new();
            for a in &artifacts {
                match delete_to_trash_dry_run(Path::new(&a.path)) {
                    Ok(()) => {
                        would_free += a.size_bytes.unwrap_or(0);
                        would_delete += 1;
                    }
                    Err(e) => rejected.push(serde_json::json!({ "path": a.path, "reason": e })),
                }
            }
            Ok(serde_json::json!({
                "dryRun": true,
                "candidates": artifacts.len(),
                "wouldDelete": would_delete,
                "wouldFreeBytes": would_free,
                "rejected": rejected,
                "note": "预演：未删除任何内容。真实执行需 dry_run=false 且 confirm=true；删除一律移入回收站（可恢复）。",
            }))
        }
        ExecMode::Real => {
            let mut freed = 0u64;
            let mut deleted = Vec::new();
            let mut failed = Vec::new();
            for a in &artifacts {
                match delete_to_trash(Path::new(&a.path)) {
                    Ok(()) => {
                        freed += a.size_bytes.unwrap_or(0);
                        deleted.push(a.path.clone());
                    }
                    Err(e) => failed.push(serde_json::json!({ "path": a.path, "error": e })),
                }
            }
            Ok(serde_json::json!({
                "dryRun": false,
                "deleted": deleted,
                "freedBytes": freed,
                "failed": failed,
                "note": "已移入回收站，可恢复；清空回收站才真正释放磁盘。",
            }))
        }
    }
}

/// deps：未使用依赖分析报告。
pub fn deps(args: &DepsArgs) -> Result<serde_json::Value, String> {
    let report = analyze_deps(Path::new(&args.project_dir))?;
    serde_json::to_value(report).map_err(|e| e.to_string())
}

/// caches：全局缓存清单。
pub fn caches() -> serde_json::Value {
    let list = discover_global_caches();
    let total: u64 = list.iter().map(|c| c.size_bytes).sum();
    serde_json::json!({
        "caches": list,
        "totalBytes": total,
        "hint": "清理用 caches_purge，传 id 列表；真实执行需 dry_run=false 且 confirm=true。",
    })
}

/// caches_purge：按 id 清理全局缓存（预演或真实）。
pub fn caches_purge(args: &PurgeArgs) -> Result<serde_json::Value, String> {
    let mode = resolve_mode("caches_purge", args.dry_run, args.confirm)?;
    if args.ids.is_empty() {
        return Err("ids 不能为空：请先调用 caches 获取可用缓存 id 列表。".into());
    }
    let dry = mode == ExecMode::DryRun;
    let mut reports = Vec::new();
    let mut failed = Vec::new();
    let mut freed = 0u64;
    for id in &args.ids {
        match purge_cache(id, dry) {
            Ok(rep) => {
                if rep.error.is_none() {
                    freed += rep.freed_bytes;
                }
                reports.push(rep);
            }
            Err(e) => failed.push(serde_json::json!({ "id": id, "error": e })),
        }
    }
    Ok(serde_json::json!({
        "dryRun": dry,
        "purged": reports,
        "freedBytes": freed,
        "failed": failed,
        "note": if dry {
            "预演：未删除任何内容。真实执行需 dry_run=false 且 confirm=true。"
        } else {
            "已移入回收站，可恢复。"
        },
    }))
}

/// archives_discover：沉睡项目发现（按最后活跃升序）。
pub fn archives_discover(args: &ArchivesArgs) -> Result<serde_json::Value, String> {
    let list = discover_archivable(
        Path::new(&args.root),
        args.stale_days.unwrap_or(0),
        &args.exclude,
    );
    let total: u64 = list.iter().map(|p| p.size_bytes).sum();
    Ok(serde_json::json!({
        "projects": list,
        "count": list.len(),
        "totalBytes": total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---------- 参数解析 ----------

    #[test]
    fn parse_scan_args_defaults() {
        // 只给 root：rules/stale_days 为 None，exclude 为空
        let args: ScanArgs = serde_json::from_value(json!({ "root": "D:/x" })).unwrap();
        assert_eq!(args.root, "D:/x");
        assert_eq!(args.rules, None);
        assert_eq!(args.stale_days, None);
        assert!(args.exclude.is_empty());
    }

    #[test]
    fn parse_scan_args_full() {
        let args: ScanArgs = serde_json::from_value(json!({
            "root": "D:/x",
            "rules": ["node", "rust"],
            "stale_days": 30,
            "exclude": ["D:/x/keep"]
        }))
        .unwrap();
        assert_eq!(args.rules.as_deref(), Some(["node".to_string(), "rust".to_string()].as_slice()));
        assert_eq!(args.stale_days, Some(30));
        assert_eq!(args.exclude, vec!["D:/x/keep"]);
    }

    #[test]
    fn parse_scan_args_missing_root_fails() {
        assert!(serde_json::from_value::<ScanArgs>(json!({ "rules": ["node"] })).is_err());
    }

    #[test]
    fn parse_clean_args_dry_run_defaults_to_none_and_confirm_false() {
        let args: CleanArgs = serde_json::from_value(json!({ "root": "D:/x" })).unwrap();
        assert_eq!(args.dry_run, None); // resolve_mode 按 true 处理
        assert!(!args.confirm);
    }

    #[test]
    fn parse_clean_args_unknown_field_rejected() {
        // 拼错的参数名（dryrun 无下划线）应当报错而不是被静默吞掉，
        // 否则 agent 以为自己在真实执行其实只是预演。
        let bad = json!({ "root": "D:/x", "dryrun": false, "confirm": true });
        assert!(serde_json::from_value::<CleanArgs>(bad).is_err());
    }

    #[test]
    fn parse_purge_args_defaults() {
        let args: PurgeArgs = serde_json::from_value(json!({ "ids": ["npm"] })).unwrap();
        assert_eq!(args.ids, vec!["npm"]);
        assert_eq!(args.dry_run, None);
        assert!(!args.confirm);
        // ids 缺省为空数组（守卫在 caches_purge 里拒绝空列表）
        let empty: PurgeArgs = serde_json::from_value(json!({})).unwrap();
        assert!(empty.ids.is_empty());
    }

    // ---------- 双开关守卫 ----------

    #[test]
    fn guard_defaults_to_dry_run() {
        assert_eq!(resolve_mode("clean", None, false).unwrap(), ExecMode::DryRun);
        assert_eq!(resolve_mode("clean", None, true).unwrap(), ExecMode::DryRun);
    }

    #[test]
    fn guard_dry_run_true_stays_preview_even_with_confirm() {
        assert_eq!(resolve_mode("clean", Some(true), true).unwrap(), ExecMode::DryRun);
    }

    #[test]
    fn guard_real_requires_both_switches() {
        // 缺 confirm → 拒绝
        let err = resolve_mode("clean", Some(false), false).unwrap_err();
        assert!(err.contains("confirm=true"), "拒绝消息应说明缺 confirm，实际: {err}");
        assert!(err.contains("未做任何修改"), "拒绝消息应说明未做修改，实际: {err}");
        // 双开关齐全 → 真实执行
        assert_eq!(resolve_mode("clean", Some(false), true).unwrap(), ExecMode::Real);
        // caches_purge 同一守卫
        assert_eq!(resolve_mode("caches_purge", Some(false), true).unwrap(), ExecMode::Real);
        assert!(resolve_mode("caches_purge", Some(false), false).is_err());
    }

    // ---------- 端到端（临时目录，走 core） ----------

    #[test]
    fn clean_end_to_end_guard_and_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("app/node_modules")).unwrap();
        std::fs::write(tmp.path().join("app/package.json"), "{}").unwrap();
        std::fs::write(tmp.path().join("app/node_modules/a.js"), "x").unwrap();

        let root = tmp.path().to_string_lossy().into_owned();

        // 1) 只传 dry_run=false、不给 confirm → 拒绝，目录原封不动
        let refused = clean(&CleanArgs {
            stale_days: None,
            dry_run: Some(false),
            confirm: false,
            root: root.clone(),
            ..Default::default()
        })
        .unwrap_err();
        assert!(refused.contains("confirm=true"));
        assert!(tmp.path().join("app/node_modules/a.js").exists());

        // 2) 默认（缺省 dry_run）→ 预演报告
        let report = clean(&CleanArgs { root: root.clone(), ..Default::default() }).unwrap();
        assert_eq!(report["dryRun"], json!(true));
        assert_eq!(report["candidates"], json!(1));
        assert_eq!(report["wouldDelete"], json!(1));
        assert!(tmp.path().join("app/node_modules/a.js").exists(), "预演不能动文件");

        // 3) excludes 覆盖删除入口：排除该项目后候选为 0
        let excluded = clean(&CleanArgs {
            root: root.clone(),
            exclude: vec![tmp.path().join("app").to_string_lossy().into_owned()],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(excluded["candidates"], json!(0));
    }

    #[test]
    fn scan_rejects_unknown_rule_id() {
        let tmp = tempfile::tempdir().unwrap();
        let err = scan_and_size(&ScanArgs {
            root: tmp.path().to_string_lossy().into_owned(),
            rules: Some(vec!["nope".into()]),
            stale_days: None,
            exclude: vec![],
        })
        .unwrap_err();
        assert!(err.contains("未知规则 id"), "实际: {err}");
        assert!(err.contains("node"), "错误信息应列出合法 id，实际: {err}");
    }

    #[test]
    fn stale_cutoff_math_is_days_in_ms() {
        // 1 天 = 86_400_000 ms；截止线不应超过"现在"
        let c = stale_cutoff_ms(Some(1)).unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        assert!(now >= c && now - c <= 86_400_000);
        assert_eq!(stale_cutoff_ms(None), None);
        // 极大天数不得下溢 panic
        assert!(stale_cutoff_ms(Some(u64::MAX)).is_some());
    }

    #[test]
    fn deps_requires_supported_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let err = deps(&DepsArgs { project_dir: tmp.path().to_string_lossy().into_owned() })
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn caches_purge_empty_ids_refused() {
        let err = caches_purge(&PurgeArgs { ids: vec![], dry_run: Some(false), confirm: true })
            .unwrap_err();
        assert!(err.contains("ids 不能为空"), "实际: {err}");
    }

    #[test]
    fn caches_list_shape() {
        let v = caches();
        assert!(v["caches"].is_array());
        assert!(v["totalBytes"].is_u64());
    }
}
