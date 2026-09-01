use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Table};
use dev_sweeper_core::{
    analyze_deps, archive_project, compute_sizes, default_archive_dir, delete_to_trash,
    detect_pypm, discover_archivable, discover_global_caches, migrate_to_pnpm, migrate_to_uv,
    prune_deps, purge_cache, restore_archive, scan_artifacts, select_rules, ArchivableProject,
    Artifact, CacheEco, CacheEntry, CachePurgeReport, DepReport, PmKind, PyPmKind,
};

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
    /// 分析项目依赖，找出未使用/多余的依赖（"重构依赖"瘦身）
    Deps {
        path: PathBuf,
        /// 确认后实际移除未使用依赖（package.json 先备份，对应 node_modules 目录移入回收站）
        #[arg(long)]
        apply: bool,
        /// 以 JSON 输出（供脚本用）
        #[arg(long)]
        json: bool,
    },
    /// 把 npm/yarn 项目迁移到 pnpm（内容寻址存储，跨项目去重以省磁盘）
    Migrate {
        path: PathBuf,
        /// 预演：只校验并报告会做什么，不移动不安装
        #[arg(long, short = 'n')]
        dry_run: bool,
        /// 跳过确认
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// 把 pip/poetry/pip-tools 项目迁移到 uv（全局内容寻址缓存 + 硬链接 venv，跨项目去重以省磁盘）
    UvMigrate {
        path: PathBuf,
        /// 预演：只校验并报告会做什么，不移动不安装
        #[arg(long, short = 'n')]
        dry_run: bool,
        /// 跳过确认
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// 扫描并清理各语言的全局依赖缓存（npm/pip/cargo/maven/gradle/go/uv/pnpm 等）
    Caches {
        /// 清理指定 id 的缓存（可重复）；不给则配合 --apply 清理全部
        #[arg(long, value_name = "ID")]
        id: Vec<String>,
        /// 清理全部已发现的全局缓存
        #[arg(long)]
        apply: bool,
        /// 预演：只报告会释放多少，不执行
        #[arg(long, short = 'n')]
        dry_run: bool,
        /// 以 JSON 输出（供脚本用）
        #[arg(long)]
        json: bool,
    },
    /// 发现沉睡项目（按最后活跃升序），用于批量压缩归档
    Archives {
        /// 根目录（扫描其下一级子目录）
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// 只显示超过 N 天未活跃的项目
        #[arg(long)]
        stale_days: Option<u64>,
        /// 以 JSON 输出（供脚本用）
        #[arg(long)]
        json: bool,
    },
    /// 把整个项目压缩归档（.tar.gz），原项目移入回收站释放工作区空间
    Archive {
        path: PathBuf,
        /// 归档库目录（默认 ~/dev-archives）
        #[arg(long)]
        archive_dir: Option<PathBuf>,
        /// 预演：只报告会做什么，不打包不删除
        #[arg(long, short = 'n')]
        dry_run: bool,
        /// 跳过确认（非交互场景）
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// 从归档解回项目
    Restore {
        /// 归档文件路径（.tar.gz）
        path: PathBuf,
        /// 还原目标根目录（默认当前目录）
        #[arg(long, default_value = ".")]
        dest: PathBuf,
        /// 预演：只校验不写
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
        Cmd::Deps { path, apply, json } => {
            let report = match analyze_deps(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("错误: {e}");
                    std::process::exit(1);
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
                return;
            }
            print_dep_report(&report);
            if !apply {
                if !report.unused.is_empty() {
                    println!("\n提示：加 --apply 可在确认后移除以上未使用依赖。");
                }
                return;
            }
            if report.unused.is_empty() {
                println!("没有可移除的未使用依赖。");
                return;
            }
            let to_remove: Vec<String> = report.unused.iter().map(|d| d.name.clone()).collect();
            println!(
                "\n将移除以下 {} 个未使用依赖（package.json 先备份为 package.json.sweep.bak，对应 node_modules 目录移入回收站）：",
                to_remove.len()
            );
            for n in &to_remove {
                println!("  - {n}");
            }
            if !confirm(to_remove.len()) {
                println!("已取消。");
                return;
            }
            match prune_deps(&path, &to_remove, false) {
                Ok(rep) => {
                    println!(
                        "\n已移除 {} 个依赖，释放 {}，备份: {}",
                        rep.removed.len(),
                        fmt_size(rep.freed_bytes),
                        rep.backup_path.unwrap_or_default()
                    );
                    if !rep.failed.is_empty() {
                        for (n, e) in &rep.failed {
                            eprintln!("  ✕ {n}: {e}");
                        }
                    }
                }
                Err(e) => eprintln!("裁剪失败: {e}"),
            }
        }
        Cmd::Migrate { path, dry_run, yes } => {
            let from = match dev_sweeper_core::detect_pm(&path) {
                PmKind::Pnpm => {
                    println!("项目已是 pnpm 管理（pnpm-lock.yaml 存在），无需迁移。");
                    return;
                }
                PmKind::Unknown => {
                    eprintln!("错误：未检测到 npm/yarn 锁文件（package-lock.json / yarn.lock），无法安全迁移。");
                    std::process::exit(1);
                }
                other => other,
            };
            let pm_name = match from {
                PmKind::Npm => "npm",
                PmKind::Yarn => "yarn",
                _ => "unknown",
            };
            if dry_run {
                println!("[dry-run] 会将 {pm_name} 项目迁移到 pnpm：");
                println!("  · 旧 node_modules 与旧锁文件移入回收站（可恢复）");
                println!("  · 运行 `pnpm import` + `pnpm install` 重建依赖");
                println!("[dry-run] 实际未执行任何修改。");
                return;
            }
            println!("将把 {pm_name} 项目迁移到 pnpm：旧 node_modules 移入回收站后立即重建（可恢复）。");
            if !yes && !confirm(1) {
                println!("已取消。");
                return;
            }
            match migrate_to_pnpm(&path, false) {
                Ok(rep) => {
                    if !rep.reinstalled {
                        eprintln!(
                            "迁移未完成（安装步骤失败）：{}",
                            rep.error.unwrap_or_default()
                        );
                        eprintln!("旧 node_modules 已移入回收站，可恢复后重试安装。");
                        std::process::exit(1);
                    }
                    println!(
                        "\n迁移完成：释放 {}，旧 node_modules 已移入回收站（{}）。",
                        fmt_size(rep.freed_bytes),
                        rep.backup_path.unwrap_or_default()
                    );
                    println!("已生成 pnpm-lock.yaml，后续用 `pnpm install` 维护。");
                }
                Err(e) => eprintln!("迁移失败: {e}"),
            }
        }
        Cmd::UvMigrate { path, dry_run, yes } => {
            let from = match detect_pypm(&path) {
                PyPmKind::Uv => {
                    println!("项目已是 uv 管理（uv.lock 存在），无需迁移。");
                    return;
                }
                PyPmKind::Unknown => {
                    eprintln!(
                        "错误：未检测到 Python 项目（需 requirements.txt / pyproject.toml / setup.py）。"
                    );
                    std::process::exit(1);
                }
                other => other,
            };
            let pm_name = match from {
                PyPmKind::Pip => "pip",
                PyPmKind::Poetry => "poetry",
                PyPmKind::PipTools => "pip-tools",
                _ => "unknown",
            };
            if dry_run {
                println!("[dry-run] 会将 {pm_name} 项目迁移到 uv：");
                println!("  · 旧 .venv 移入回收站（可恢复）");
                println!("  · 运行 `uv venv` + 按清单安装（pyproject→uv sync / requirements→uv pip install）");
                println!("[dry-run] 实际未执行任何修改。");
                return;
            }
            println!("将把 {pm_name} 项目迁移到 uv：旧 .venv 移入回收站后立即重建（可恢复）。");
            if !yes && !confirm(1) {
                println!("已取消。");
                return;
            }
            match migrate_to_uv(&path, false) {
                Ok(rep) => {
                    if !rep.reinstalled {
                        eprintln!(
                            "迁移未完成（安装步骤失败）：{}",
                            rep.error.unwrap_or_default()
                        );
                        eprintln!("旧 .venv 已移入回收站，可恢复后重试安装。");
                        std::process::exit(1);
                    }
                    println!(
                        "\n迁移完成：释放 {}，旧 .venv 已移入回收站（{}）。",
                        fmt_size(rep.freed_bytes),
                        rep.backup_path.unwrap_or_default()
                    );
                    println!("已建立 uv 管理的 .venv；后续用 `uv sync` / `uv pip install` 维护。");
                }
                Err(e) => eprintln!("迁移失败: {e}"),
            }
        }
        Cmd::Caches { id, apply, dry_run, json } => {
            let caches = discover_global_caches();
            if caches.is_empty() {
                println!("未发现任何全局依赖缓存。");
                return;
            }
            if !id.is_empty() || apply {
                let targets: Vec<String> = if id.is_empty() {
                    caches.iter().map(|c| c.id.clone()).collect()
                } else {
                    id.clone()
                };
                let known: std::collections::HashSet<String> =
                    caches.iter().map(|c| c.id.clone()).collect();
                for t in &targets {
                    if !known.contains(t) {
                        eprintln!("未知缓存 id: {t}（用 `sweep caches` 查看可用 id）");
                        return;
                    }
                }
                if dry_run {
                    let total: u64 = caches
                        .iter()
                        .filter(|c| targets.contains(&c.id))
                        .map(|c| c.size_bytes)
                        .sum();
                    println!(
                        "[dry-run] 会清理 {} 个全局缓存，释放 {}",
                        targets.len(),
                        fmt_size(total)
                    );
                    println!("[dry-run] 实际未删除任何内容。");
                    return;
                }
                if !json {
                    println!("将清理 {} 个全局缓存（移入回收站，可恢复）。", targets.len());
                    if !confirm(targets.len()) {
                        println!("已取消。");
                        return;
                    }
                }
                let mut reports: Vec<CachePurgeReport> = Vec::new();
                let mut freed = 0u64;
                let mut failed = 0usize;
                for t in &targets {
                    match purge_cache(t, false) {
                        Ok(rep) => {
                            if let Some(e) = &rep.error {
                                failed += 1;
                                eprintln!("  ✕ {t}: {e}");
                            } else {
                                freed += rep.freed_bytes;
                            }
                            reports.push(rep);
                        }
                        Err(e) => {
                            failed += 1;
                            eprintln!("  ✕ {t}: {e}");
                        }
                    }
                }
                if json {
                    println!("{}", serde_json::to_string_pretty(&reports).unwrap());
                } else {
                    println!(
                        "\n共释放 {}（{} 项失败），可在回收站恢复。",
                        fmt_size(freed),
                        failed
                    );
                }
            } else {
                if json {
                    println!("{}", serde_json::to_string_pretty(&caches).unwrap());
                } else {
                    print_cache_list(&caches);
                }
            }
        }
        Cmd::Archives { root, stale_days, json } => {
            let list = discover_archivable(Path::new(&root), stale_days.unwrap_or(0));
            if json {
                println!("{}", serde_json::to_string_pretty(&list).unwrap());
            } else {
                print_archivable_list(&list);
            }
        }
        Cmd::Archive {
            path,
            archive_dir,
            dry_run,
            yes,
        } => {
            let dir = archive_dir
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(default_archive_dir);
            if dry_run {
                match archive_project(Path::new(&path), Path::new(&dir), true) {
                    Ok(r) => println!(
                        "[dry-run] 会将 {} 压缩为 {}，原项目移入回收站（释放 {}），实际未执行。",
                        r.source_path, r.archive_file, fmt_size(r.original_size)
                    ),
                    Err(e) => eprintln!("错误: {e}"),
                }
                return;
            }
            println!(
                "将把 {} 压缩为 {}/<name>@<日期>.tar.gz，原项目移入回收站（可恢复）。",
                path.display(),
                dir
            );
            if !yes && !confirm(1) {
                println!("已取消。");
                return;
            }
            match archive_project(Path::new(&path), Path::new(&dir), false) {
                Ok(r) => {
                    if r.removed_original {
                        println!(
                            "\n已归档 {}：原始 {} → 压缩 {}，原项目已移入回收站（{}）。",
                            r.name,
                            fmt_size(r.original_size),
                            fmt_size(r.compressed_size),
                            r.archive_file
                        );
                    } else {
                        println!(
                            "\n已生成归档 {}（{}），但原项目移入回收站失败：{}",
                            r.archive_file,
                            fmt_size(r.compressed_size),
                            r.error.unwrap_or_default()
                        );
                    }
                    println!(
                        "提示：清空回收站即可真正释放 {}；归档可随时用 `sweep restore` 解回。",
                        fmt_size(r.original_size)
                    );
                }
                Err(e) => eprintln!("归档失败: {e}"),
            }
        }
        Cmd::Restore {
            path,
            dest,
            dry_run,
        } => {
            if dry_run {
                match restore_archive(Path::new(&path), Path::new(&dest), true) {
                    Ok(r) => println!(
                        "[dry-run] 会将 {} 解回到 {}，实际未写入。",
                        r.archive_file, r.restored_to
                    ),
                    Err(e) => eprintln!("错误: {e}"),
                }
                return;
            }
            match restore_archive(Path::new(&path), Path::new(&dest), false) {
                Ok(r) => println!(
                    "\n已解回 {} → {}（{}），原归档保留。",
                    r.archive_file,
                    r.restored_to,
                    fmt_size(r.restored_bytes)
                ),
                Err(e) => eprintln!("还原失败: {e}"),
            }
        }
    }
}

/// 渲染依赖分析报告（未使用 + 多余）。
fn print_dep_report(report: &DepReport) {
    println!(
        "项目 {}（{}）：声明 {} 个依赖，其中 {} 个被源码引用。",
        report.project_name, report.project_dir, report.declared_count, report.used_count
    );

    if report.unused.is_empty() {
        println!("\n✓ 未发现未使用的声明依赖。");
    } else {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_header(["依赖", "类别", "置信", "状态", "说明"]);
        for d in &report.unused {
            let kind = match d.kind {
                dev_sweeper_core::DepKind::Runtime => "运行",
                dev_sweeper_core::DepKind::Dev => "开发",
            };
            let conf = match d.confidence {
                dev_sweeper_core::DepConfidence::High => "高",
                dev_sweeper_core::DepConfidence::Review => "需复核",
            };
            let status = match d.status {
                dev_sweeper_core::DepStatus::Unused => "未使用",
                _ => "?",
            };
            table.add_row([
                d.name.clone(),
                kind.into(),
                conf.into(),
                status.into(),
                d.note.clone().unwrap_or_default(),
            ]);
        }
        println!("\n未使用的声明依赖（候选移除）：");
        println!("{table}");
    }

    if !report.extraneous.is_empty() {
        let names: Vec<&str> = report.extraneous.iter().map(|d| d.name.as_str()).collect();
        println!(
            "\nnode_modules 中不在 package.json 的目录（{} 个，可能含传递依赖，建议 npm prune 为准）：",
            report.extraneous.len()
        );
        println!("  {}", names.join(", "));
    }

    for n in &report.notes {
        println!("\n· {n}");
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

fn print_archivable_list(list: &[ArchivableProject]) {
    if list.is_empty() {
        println!("未发现沉睡项目（或全部都很新鲜）。");
        return;
    }
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["项目", "大小", "最后活跃", "git", "路径"]);
    for p in list {
        table.add_row([
            p.name.clone(),
            fmt_size(p.size_bytes),
            fmt_days_ago(p.last_active_ms),
            (if p.is_git { "✓" } else { "" }).into(),
            p.path.clone(),
        ]);
    }
    println!("{table}");
    let total: u64 = list.iter().map(|p| p.size_bytes).sum();
    println!(
        "共 {} 个沉睡项目，合计 {}（按最后活跃升序，最旧的在前）",
        list.len(),
        fmt_size(total)
    );
    println!("提示：用 `sweep archive <path>` 把单个项目压成 .tar.gz 归档并移入回收站。");
}

fn print_cache_list(caches: &[CacheEntry]) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["id", "生态", "缓存", "大小", "风险", "路径"]);
    for c in caches {
        let eco = match c.eco {
            CacheEco::Node => "Node",
            CacheEco::Rust => "Rust",
            CacheEco::Java => "Java",
            CacheEco::Go => "Go",
            CacheEco::Python => "Python",
            CacheEco::Pnpm => "pnpm",
            CacheEco::Unknown => "?",
        };
        let risk = match c.risk.as_str() {
            "safe" => "🟢 安全",
            "notice" => "🟡 注意",
            _ => "?",
        };
        table.add_row([
            c.id.clone(),
            eco.into(),
            c.label.clone(),
            fmt_size(c.size_bytes),
            risk.into(),
            c.path.clone(),
        ]);
    }
    println!("{table}");
    let total: u64 = caches.iter().map(|c| c.size_bytes).sum();
    println!("共 {} 个全局缓存，合计 {}", caches.len(), fmt_size(total));
    println!("提示：加 --apply 清理全部，或 --id <id> 清理指定项；-n 预演。");
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
