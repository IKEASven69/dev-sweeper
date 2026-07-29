use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Table};
use dev_sweeper_core::{compute_sizes, delete_to_trash, scan_artifacts, select_rules, Artifact};

/// CLI 不做优雅取消——用户 Ctrl+C 直接终止进程即可。
/// 这里只是提供一个永不置位的标志，统一 scan_artifacts 的调用签名。
fn never_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[derive(Parser)]
#[command(
    name = "sweep",
    version,
    about = "扫描并清理开发产物（node_modules、target、venv…），删除一律移入回收站"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 扫描并列出产物目录
    Scan {
        path: PathBuf,
        /// 规则过滤，逗号分隔（node,rust,maven,gradle,python-venv,python-cache,web-dist）
        #[arg(long, value_delimiter = ',')]
        rules: Vec<String>,
        /// 只列出超过 N 天未活跃的项目
        #[arg(long)]
        stale_days: Option<u64>,
        /// 以 JSON 输出（供脚本用）
        #[arg(long)]
        json: bool,
        /// 排除（保护）路径前缀，逗号分隔，可多次。命中的产物不扫描
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<String>,
        /// 跳过大小计算（快速列出产物，不显示体积）
        #[arg(long)]
        no_size: bool,
    },
    /// 扫描并把产物目录移入回收站
    Clean {
        path: PathBuf,
        #[arg(long, value_delimiter = ',')]
        rules: Vec<String>,
        #[arg(long)]
        stale_days: Option<u64>,
        /// 排除（保护）路径前缀，逗号分隔，可多次。命中的产物不扫描
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<String>,
        /// 跳过大小计算（clean 时仍会正常删除，仅表格不显示体积）
        #[arg(long)]
        no_size: bool,
        /// 跳过确认
        #[arg(long, short = 'y')]
        yes: bool,
        /// 预演：只校验不执行，显示"本会移入回收站"的清单
        #[arg(long, short = 'n')]
        dry_run: bool,
    },
}

fn main() {
    match Cli::parse().cmd {
        Cmd::Scan { path, rules, stale_days, json, exclude, no_size } => {
            let artifacts = scan_and_size(&path, &rules, stale_days, &exclude, no_size);
            if json {
                println!("{}", serde_json::to_string_pretty(&artifacts).unwrap());
            } else {
                print_table(&artifacts);
            }
        }
        Cmd::Clean { path, rules, stale_days, exclude, no_size, yes, dry_run } => {
            let artifacts = scan_and_size(&path, &rules, stale_days, &exclude, no_size);
            if artifacts.is_empty() {
                println!("没有可清理的产物。");
                return;
            }
            print_table(&artifacts);
            if dry_run {
                run_dry_run(&artifacts);
                return;
            }
            if !yes && !confirm(artifacts.len()) {
                println!("已取消。");
                return;
            }
            let mut freed = 0u64;
            let mut failed = 0usize;
            for a in &artifacts {
                match delete_to_trash(Path::new(&a.path)) {
                    Ok(()) => {
                        freed += a.size_bytes.unwrap_or(0);
                        println!("已移入回收站  {}", a.path);
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("失败  {e}");
                    }
                }
            }
            println!("\n共释放 {}（{} 项失败），可在回收站恢复。", fmt_size(freed), failed);
        }
    }
}

fn scan_and_size(
    path: &Path,
    rule_ids: &[String],
    stale_days: Option<u64>,
    excludes: &[String],
    skip_size: bool,
) -> Vec<Artifact> {
    // #6: 未知规则 id 打 warning，避免用户以为在扫某类产物实际没扫
    if !rule_ids.is_empty() {
        let known: std::collections::HashSet<&str> = dev_sweeper_core::RULES
            .iter()
            .map(|r| r.id)
            .collect();
        for id in rule_ids {
            if !known.contains(id.as_str()) {
                let all_ids: Vec<&str> = dev_sweeper_core::RULES.iter().map(|r| r.id).collect();
                eprintln!("warning: 未知规则 id「{id}」（已忽略）。已知: {}", all_ids.join(", "));
            }
        }
    }
    let rules = select_rules(rule_ids);
    let cancel = never_cancel();
    eprintln!("扫描 {} …", path.display());
    let mut artifacts = scan_artifacts(path, &rules, &cancel, excludes, |_| {}, |_| {});
    if !skip_size {
        compute_sizes(&mut artifacts, &cancel, |_, _| {});
    }
    if let Some(days) = stale_days {
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let cutoff = now_ms.saturating_sub(days * 24 * 3600 * 1000);
        artifacts.retain(|a| a.last_active_ms.is_some_and(|t| t < cutoff));
    }
    // skip_size 时 size_bytes 为 None，排序时退化为按扫描顺序稳定排
    artifacts.sort_by_key(|a| std::cmp::Reverse(a.size_bytes.unwrap_or(0)));
    artifacts
}

/// dry-run：对每条产物跑 marker + 存在性校验，不真正删除。
/// 统计"可移入回收站"的项数与体积，并列出被校验拒绝的项。
fn run_dry_run(artifacts: &[Artifact]) {
    use dev_sweeper_core::delete_to_trash_dry_run;
    let mut would_free = 0u64;
    let mut would_delete = 0usize;
    let mut rejected = Vec::new();
    for a in artifacts {
        match delete_to_trash_dry_run(Path::new(&a.path)) {
            Ok(()) => {
                would_free += a.size_bytes.unwrap_or(0);
                would_delete += 1;
            }
            Err(e) => rejected.push((a.path.clone(), e)),
        }
    }
    println!("\n[dry-run] 本会移入回收站 {would_delete} 项，释放 {}", fmt_size(would_free));
    println!("[dry-run] 实际未删除任何内容。");
    if !rejected.is_empty() {
        eprintln!("\n[dry-run] {} 项未通过校验：", rejected.len());
        for (p, e) in &rejected {
            eprintln!("  {p}  → {e}");
        }
    }
}

fn print_table(artifacts: &[Artifact]) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["类型", "风险", "项目", "大小", "最后活跃", "路径"]);
    for a in artifacts {
        let risk = match a.risk.as_str() {
            "safe" => "🟢 安全",
            "notice" => "🟡 注意",
            _ => "?",
        };
        table.add_row([
            a.rule_id.clone(),
            risk.into(),
            a.project_name.clone(),
            fmt_size(a.size_bytes.unwrap_or(0)),
            fmt_days_ago(a.last_active_ms),
            a.path.clone(),
        ]);
    }
    println!("{table}");
    let total: u64 = artifacts.iter().filter_map(|a| a.size_bytes).sum();
    println!("共 {} 项，合计 {}", artifacts.len(), fmt_size(total));
}

fn confirm(count: usize) -> bool {
    print!("将把 {count} 个目录移入回收站（可恢复），确认? [y/N] ");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap_or(0);
    matches!(line.trim(), "y" | "Y" | "yes")
}

fn fmt_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

fn fmt_days_ago(last_active_ms: Option<u64>) -> String {
    let Some(t) = last_active_ms else {
        return "未知".into();
    };
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let days = now_ms.saturating_sub(t) / (24 * 3600 * 1000);
    if days == 0 {
        "今天".into()
    } else {
        format!("{days} 天前")
    }
}
