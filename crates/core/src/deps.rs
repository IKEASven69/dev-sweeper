//! 依赖裁剪（"重构依赖"）分析模块。
//!
//! 与扫描产物（`scan.rs`）互补：后者关心"占磁盘的构建/依赖产物"，本模块关心
//! "依赖清单本身是否臃肿"——找出 `package.json` 里**声明了但源码从未引用**的依赖，
//! 让用户精准瘦身，而不是整包 `node_modules` 全删（那样项目就跑不起来了）。
//!
//! 安全边界（与 `delete.rs` 一致）：
//! - 分析只读不写，不启动任何子进程（保持 core 无 IO 框架）。
//! - 裁剪（`prune_deps`）只改写 `package.json` 并写一份 `*.sweep.bak` 备份，
//!   同时把对应 `node_modules/<pkg>` 目录**移入回收站**（可恢复），永不 `remove_dir_all`。
//!
//! 现状：v1 聚焦 Node 生态（`package.json` + 源码 import 扫描）。其他生态（Rust/Python…）
//! 的"未使用依赖"判定方式不同（cargo udeps / pip-autoremove），后续按本模块结构扩展。

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

/// 生态类型。v1 仅 Node，其余标记 Unknown 并在 analyze 时给出可读错误。
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Eco {
    Node,
    Unknown,
}

/// 包管理器类型（按锁文件识别）。
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PmKind {
    Npm,
    Yarn,
    Pnpm,
    /// 无 package-lock.json / yarn.lock / pnpm-lock.yaml，无法安全迁移
    Unknown,
}

/// 按锁文件识别当前包管理器。
///
/// 优先级：pnpm-lock.yaml > yarn.lock > package-lock.json（部分迁移的项目若两种锁文件并存，以 pnpm 为准）。
pub fn detect_pm(dir: &Path) -> PmKind {
    if dir.join("pnpm-lock.yaml").is_file() {
        PmKind::Pnpm
    } else if dir.join("yarn.lock").is_file() {
        PmKind::Yarn
    } else if dir.join("package-lock.json").is_file() {
        PmKind::Npm
    } else {
        PmKind::Unknown
    }
}

/// 依赖类别：运行期（dependencies）还是开发期（devDependencies）。
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DepKind {
    Runtime,
    Dev,
}

/// 依赖状态：被引用 / 声明了未引用（候选移除） / 多余（node_modules 里有但清单没有）。
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DepStatus {
    Used,
    Unused,
    Extraneous,
}

/// 移除建议的置信度：
/// - High：几乎可以确定没被用到（运行期依赖从未被 import）。
/// - Review：需人工复核（开发期依赖常通过 CLI/配置文件使用，未必出现在源码 import 中）。
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DepConfidence {
    High,
    Review,
}

/// 单条依赖的分析结果。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DepEntry {
    pub name: String,
    pub version: Option<String>,
    pub kind: DepKind,
    pub status: DepStatus,
    pub confidence: DepConfidence,
    /// 参考说明（如"开发依赖，可能仅通过 CLI 使用"）。
    pub note: Option<String>,
}

/// 项目依赖分析报告。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DepReport {
    pub eco: Eco,
    /// 包管理器（按锁文件识别：npm / yarn / pnpm / unknown）。
    pub pm: PmKind,
    pub project_dir: String,
    pub project_name: String,
    /// 声明依赖总数（dependencies + devDependencies）。
    pub declared_count: usize,
    /// 被源码引用到的声明依赖数。
    pub used_count: usize,
    /// 声明了但源码未引用的依赖（候选移除）。
    pub unused: Vec<DepEntry>,
    /// node_modules 中存在但不在 package.json 中的目录（建议 npm prune）。
    pub extraneous: Vec<DepEntry>,
    pub notes: Vec<String>,
}

/// 裁剪（移除依赖）的执行报告。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PruneReport {
    /// 实际从 package.json 移除的依赖名。
    pub removed: Vec<String>,
    /// 移入回收站的 node_modules 子目录释放的字节数。
    pub freed_bytes: u64,
    /// 备份文件路径（package.json.sweep.bak）；dry_run 或未改动时为 None。
    pub backup_path: Option<String>,
    /// (依赖名, 失败原因) 列表。
    pub failed: Vec<(String, String)>,
    pub dry_run: bool,
}

/// 识别项目生态。v1：含 package.json 即视为 Node。
pub fn detect_eco(dir: &Path) -> Eco {
    if dir.join("package.json").is_file() {
        Eco::Node
    } else {
        Eco::Unknown
    }
}

/// 分析项目依赖。仅支持 Node（含 package.json）。
///
/// 返回 `Err` 当目录不含受支持的清单文件。
pub fn analyze_deps(dir: &Path) -> Result<DepReport, String> {
    match detect_eco(dir) {
        Eco::Node => Ok(analyze_node(dir)),
        Eco::Unknown => Err(format!(
            "未识别到支持的项目：需要在目录中找到 package.json（当前仅支持 Node 生态）"
        )),
    }
}

fn project_name_of(dir: &Path) -> String {
    if let Ok(text) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
                return name.to_string();
            }
        }
    }
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string_lossy().into_owned())
}

fn analyze_node(dir: &Path) -> DepReport {
    let empty = serde_json::Value::Null;
    let manifest_text = std::fs::read_to_string(dir.join("package.json")).unwrap_or_default();
    let manifest = serde_json::from_str::<serde_json::Value>(&manifest_text).unwrap_or(empty);

    // 收集声明依赖：name -> (kind, version)
    let mut declared: Vec<(String, DepKind, Option<String>)> = Vec::new();
    if let Some(deps) = manifest.get("dependencies").and_then(|v| v.as_object()) {
        for (name, ver) in deps {
            declared.push((name.clone(), DepKind::Runtime, ver.as_str().map(String::from)));
        }
    }
    if let Some(dev) = manifest.get("devDependencies").and_then(|v| v.as_object()) {
        for (name, ver) in dev {
            declared.push((name.clone(), DepKind::Dev, ver.as_str().map(String::from)));
        }
    }

    // 扫描源码 import，得到"被引用"的顶层包名集合
    let used: HashSet<String> = scan_imported_packages(dir);

    let declared_names: HashSet<String> = declared.iter().map(|(n, _, _)| n.clone()).collect();

    // 逐条判定状态
    let mut unused: Vec<DepEntry> = Vec::new();
    let mut used_count = 0usize;
    for (name, kind, version) in &declared {
        let is_used = used.contains(name);
        if is_used {
            used_count += 1;
            continue; // 只关心未使用的，Used 不进列表
        }
        let (confidence, note) = match kind {
            DepKind::Runtime => (
                DepConfidence::High,
                Some("运行期依赖，源码中从未 import，几乎可以确定移除".to_string()),
            ),
            DepKind::Dev => (
                DepConfidence::Review,
                Some("开发依赖，可能仅通过 CLI / 配置文件（如 eslint、vite、tsc）使用，未在源码 import 中出现".to_string()),
            ),
        };
        unused.push(DepEntry {
            name: name.clone(),
            version: version.clone(),
            kind: *kind,
            status: DepStatus::Unused,
            confidence,
            note,
        });
    }
    // 稳定排序：先 High 后 Review，同名按字母
    let rank = |c: &DepConfidence| match c {
        DepConfidence::High => 0u8,
        DepConfidence::Review => 1u8,
    };
    unused.sort_by(|a, b| rank(&a.confidence).cmp(&rank(&b.confidence)).then(a.name.cmp(&b.name)));

    // 多余依赖：node_modules 顶层目录不在声明清单中的（可能含传递依赖，需以 npm prune 为准）
    let mut extraneous: Vec<DepEntry> = Vec::new();
    let nm = dir.join("node_modules");
    if nm.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&nm) {
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().into_owned();
                // 跳过相对/隐藏/内部目录
                if fname.starts_with('.') || fname == "node_modules" {
                    continue;
                }
                // @scope 作用域目录本身不是包，跳过（内部子包无法在此判定）
                if fname.starts_with('@') {
                    continue;
                }
                if declared_names.contains(&fname) {
                    continue;
                }
                extraneous.push(DepEntry {
                    name: fname.clone(),
                    version: None,
                    kind: DepKind::Runtime,
                    status: DepStatus::Extraneous,
                    confidence: DepConfidence::Review,
                    note: Some("node_modules 中存在但不在 package.json；可能为其依赖的传递依赖，请以 `npm prune` 为准".to_string()),
                });
            }
        }
    }
    extraneous.sort_by(|a, b| a.name.cmp(&b.name));

    let mut notes = Vec::new();
    if !unused.is_empty() {
        notes.push("未使用依赖为候选移除项；运行期依赖（High）可较放心移除，开发依赖（Review）请先确认未被 CLI/配置引用。".into());
    }
    if !extraneous.is_empty() {
        notes.push(format!(
            "检测到 {} 个 node_modules 顶层目录不在 package.json 中；其中可能包含合法的传递依赖，建议运行 `npm prune` / `pnpm prune` 让包管理器权威判定。",
            extraneous.len()
        ));
    }
    if unused.is_empty() && extraneous.is_empty() {
        notes.push("未发现明显可裁剪的依赖，干得漂亮 🎉".into());
    }

    DepReport {
        eco: Eco::Node,
        pm: detect_pm(dir),
        project_dir: dir.to_string_lossy().into_owned(),
        project_name: project_name_of(dir),
        declared_count: declared.len(),
        used_count,
        unused,
        extraneous,
        notes,
    }
}

/// 遍历项目源码目录，提取所有被 import/require 的顶层包名。
///
/// 跳过 node_modules、.git 及常见构建产物目录，避免扫到依赖自身代码。
fn scan_imported_packages(dir: &Path) -> HashSet<String> {
    let mut used = HashSet::new();
    let skip_dirs: HashSet<&str> = [
        "node_modules",
        ".git",
        "target",
        "dist",
        "build",
        ".next",
        ".venv",
        "venv",
        "__pycache__",
        ".cache",
        "coverage",
    ]
    .into_iter()
    .collect();

    let walker = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir() && skip_dirs.contains(name.as_ref()))
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_source_file(path) {
            continue;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for spec in extract_import_specifiers(&text) {
            if let Some(pkg) = resolve_package_name(&spec) {
                used.insert(pkg);
            }
        }
    }
    used
}

/// 是否是需要扫描的源码文件（按扩展名）。
fn is_source_file(path: &Path) -> bool {
    static EXTS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs", "vue", "svelte"];
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// 从一段源码文本中提取所有 import/require 的字符串字面量（模块说明符）。
pub fn extract_import_specifiers(text: &str) -> Vec<String> {
    let patterns: &[&str] = &[
        // import ... from 'x'  /  export ... from 'x'
        r#"(?:import|export)\b[^;]*?\bfrom\s*['"]([^'"]+)['"]"#,
        // import 'x'  /  import("x")  /  require('x')
        r#"(?:import|require)\s*\(\s*['"]([^'"]+)['"]\s*\)"#,
        // 副作用导入 import 'x'
        r#"import\s+['"]([^'"]+)['"]"#,
    ];
    let mut out = Vec::new();
    for pat in patterns {
        // 编译失败不可能（模式为常量），unwrap 安全
        let re = regex::Regex::new(pat).expect("静态正则模式必须有效");
        for cap in re.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                out.push(m.as_str().to_string());
            }
        }
    }
    // 去重
    out.sort();
    out.dedup();
    out
}

/// 把模块说明符解析为顶层包名。
///
/// - 相对/绝对路径、URL、node: 内置模块 → None（不计入依赖）。
/// - `@scope/name` / `@scope/name/sub` → `@scope/name`。
/// - `name` / `name/sub` → `name`。
pub fn resolve_package_name(spec: &str) -> Option<String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    // 相对/绝对路径
    if spec.starts_with('.') || spec.starts_with('/') || spec.starts_with('\\') {
        return None;
    }
    // 协议 / node: 内置
    if spec.contains("://")
        || spec.starts_with("node:")
        || spec.starts_with("file:")
        || spec.starts_with("data:")
        || spec.starts_with('#')
    {
        return None;
    }
    let parts: Vec<&str> = spec.split('/').collect();
    if parts.is_empty() {
        return None;
    }
    if spec.starts_with('@') {
        // scope 包：取前两段
        if parts.len() >= 2 {
            Some(format!("{}/{}", parts[0], parts[1]))
        } else {
            None
        }
    } else {
        Some(parts[0].to_string())
    }
}

/// 裁剪（移除）依赖。
///
/// - `remove`：要移除的依赖名（应来自 `DepReport.unused`）。
/// - 从 `package.json` 的 dependencies / devDependencies 中删除这些项，
///   写一份 `package.json.sweep.bak` 备份，再把对应 `node_modules/<pkg>` 目录移入回收站。
/// - `dry_run = true` 时只校验、不写文件、不移动，返回"本会移除"的清单。
pub fn prune_deps(
    dir: &Path,
    remove: &[String],
    dry_run: bool,
) -> Result<PruneReport, String> {
    if detect_eco(dir) != Eco::Node {
        return Err("仅支持 Node 项目（需 package.json）".into());
    }
    let manifest_path = dir.join("package.json");
    let original = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("无法读取 package.json: {e}"))?;
    let mut manifest: serde_json::Value = serde_json::from_str(&original)
        .map_err(|e| format!("package.json 不是合法 JSON: {e}"))?;

    let remove_set: HashSet<&String> = remove.iter().collect();
    let mut removed: Vec<String> = Vec::new();

    for field in ["dependencies", "devDependencies"] {
        if let Some(obj) = manifest.get_mut(field).and_then(|v| v.as_object_mut()) {
            let names: Vec<String> = obj.keys().cloned().collect();
            for name in names {
                if remove_set.contains(&name) {
                    obj.remove(&name);
                    if !removed.contains(&name) {
                        removed.push(name);
                    }
                }
            }
            // 清空后删除空字段，保持清单整洁
            if obj.is_empty() {
                if let Some(m) = manifest.as_object_mut() {
                    m.remove(field);
                }
            }
        }
    }

    if removed.is_empty() {
        return Ok(PruneReport {
            removed: Vec::new(),
            freed_bytes: 0,
            backup_path: None,
            failed: Vec::new(),
            dry_run,
        });
    }

    let backup_path = if dry_run {
        None
    } else {
        let bak = dir.join("package.json.sweep.bak");
        std::fs::write(&bak, &original).map_err(|e| format!("写入备份失败: {e}"))?;
        Some(bak.to_string_lossy().into_owned())
    };

    let new_text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("序列化 package.json 失败: {e}"))?;
    if !dry_run {
        std::fs::write(&manifest_path, new_text).map_err(|e| format!("写回 package.json 失败: {e}"))?;
    }

    // 移动对应 node_modules 目录进回收站，立即释放磁盘（可恢复）
    let mut freed_bytes = 0u64;
    let mut failed = Vec::new();
    let nm = dir.join("node_modules");
    for name in &removed {
        let pkg_dir = nm.join(name);
        if !pkg_dir.is_dir() {
            continue;
        }
        if dry_run {
            freed_bytes += crate::scan::dir_size(&pkg_dir);
            continue;
        }
        let size = crate::scan::dir_size(&pkg_dir);
        match trash::delete(&pkg_dir) {
            Ok(()) => freed_bytes += size,
            Err(e) => failed.push((name.clone(), format!("移入回收站失败: {e}"))),
        }
    }

    Ok(PruneReport {
        removed,
        freed_bytes,
        backup_path,
        failed,
        dry_run,
    })
}

/// 把 npm / yarn 项目迁移到 pnpm 的执行报告。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MigrateReport {
    /// 迁移前的包管理器（来自锁文件识别）。
    pub from_pm: PmKind,
    /// 旧 node_modules 移入回收站释放的体积（字节）。
    pub freed_bytes: u64,
    /// 被移入回收站的旧 node_modules 原路径（用于提示用户"已回收站可恢复"）。
    pub backup_path: Option<String>,
    /// 是否成功执行 pnpm import + pnpm install。
    pub reinstalled: bool,
    /// 迁移过程错误（若有）。
    pub error: Option<String>,
    /// 是否仅演练。
    pub dry_run: bool,
}

/// 把 npm / yarn 项目迁移到 pnpm。
///
/// pnpm 使用**内容寻址全局存储**（`~/.pnpm-store`），多个项目的同一份包只存一份，
/// 相比 npm 扁平 `node_modules` 能显著省磁盘——这是 dev-sweeper 扫到大量项目时的核心省盘杠杆。
///
/// 步骤：
/// 1. 仅当检测到 npm / yarn 锁文件才允许迁移（Unknown 拒绝；已是 pnpm 直接拒绝）。
/// 2. `dry_run = true` 时只报告"会做什么"，不移动、不安装。
/// 3. 非空跑：先把旧 `node_modules` 与旧锁文件（package-lock.json / yarn.lock）**移入回收站**
///    以立即释放磁盘，再运行 `pnpm import`（基于已有 lockfile 生成 pnpm-lock.yaml）+ `pnpm install`。
///
/// 安全边界：与 `prune_deps` 一致，删除一律走回收站（`trash::delete`），永不 `remove_dir_all`；
/// 安装失败不影响已被回收站保护的旧 `node_modules`，用户可随时恢复。
pub fn migrate_to_pnpm(dir: &Path, dry_run: bool) -> Result<MigrateReport, String> {
    if detect_eco(dir) != Eco::Node {
        return Err("仅支持 Node 项目（需 package.json）".into());
    }
    let from_pm = detect_pm(dir);
    if from_pm == PmKind::Pnpm {
        return Err("项目已是 pnpm 管理（存在 pnpm-lock.yaml），无需迁移".into());
    }
    if from_pm == PmKind::Unknown {
        return Err(
            "未检测到 npm/yarn 锁文件（package-lock.json / yarn.lock），无法安全迁移到 pnpm"
                .into(),
        );
    }

    if dry_run {
        return Ok(MigrateReport {
            from_pm,
            freed_bytes: 0,
            backup_path: None,
            reinstalled: false,
            error: None,
            dry_run,
        });
    }

    // 1) 备份旧 node_modules 与旧锁文件到回收站，立即释放磁盘（可恢复）
    let nm = dir.join("node_modules");
    let old_lock = match from_pm {
        PmKind::Npm => Some("package-lock.json"),
        PmKind::Yarn => Some("yarn.lock"),
        _ => None,
    };
    let mut freed_bytes = 0u64;
    let mut backup_path: Option<String> = None;

    if nm.is_dir() {
        freed_bytes += crate::scan::dir_size(&nm);
        match trash::delete(&nm) {
            Ok(()) => backup_path = Some(nm.to_string_lossy().into_owned()),
            Err(e) => {
                return Ok(MigrateReport {
                    from_pm,
                    freed_bytes: 0,
                    backup_path: None,
                    reinstalled: false,
                    error: Some(format!("无法将旧 node_modules 移入回收站: {e}")),
                    dry_run,
                });
            }
        }
    }
    if let Some(name) = old_lock {
        let lock = dir.join(name);
        if lock.is_file() {
            freed_bytes += std::fs::metadata(&lock).map(|m| m.len()).unwrap_or(0);
            // 旧锁文件移入回收站失败不致命（pnpm 会另写 pnpm-lock.yaml），仅忽略
            let _ = trash::delete(&lock);
        }
    }

    // 2) 运行 pnpm import && pnpm install（优先全局 pnpm，回退 npx pnpm）
    let result = run_pnpm(&["import"], dir).and_then(|_| run_pnpm(&["install"], dir));
    let reinstalled = result.is_ok();
    let error = result.err();

    Ok(MigrateReport {
        from_pm,
        freed_bytes,
        backup_path,
        reinstalled,
        error,
        dry_run,
    })
}

/// 运行 pnpm 子命令；优先 `pnpm`，spawn 失败（未安装）时回退 `npx --yes pnpm`。
fn run_pnpm(args: &[&str], dir: &Path) -> Result<(), String> {
    let full: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let candidates: Vec<(String, Vec<String>)> = vec![
        ("pnpm".to_string(), full.clone()),
        (
            "npx".to_string(),
            {
                let mut v = vec!["--yes".to_string(), "pnpm".to_string()];
                v.extend(full.iter().cloned());
                v
            },
        ),
    ];
    let mut last_err = String::new();
    for (cmd, cmd_args) in candidates {
        match std::process::Command::new(&cmd)
            .args(&cmd_args)
            .current_dir(dir)
            .status()
        {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => last_err = format!("`{} {}` 退出码 {}; ", cmd, cmd_args.join(" "), s),
            Err(e) => last_err = format!("无法执行 `{}`: {}; ", cmd, e),
        }
    }
    Err(format!("pnpm 执行失败: {}", last_err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn resolve_package_name_basics() {
        assert_eq!(resolve_package_name("lodash"), Some("lodash".into()));
        assert_eq!(resolve_package_name("lodash/sub"), Some("lodash".into()));
        assert_eq!(resolve_package_name("@scope/pkg"), Some("@scope/pkg".into()));
        assert_eq!(resolve_package_name("@scope/pkg/sub/x"), Some("@scope/pkg".into()));
        assert_eq!(resolve_package_name("./local"), None);
        assert_eq!(resolve_package_name("../up"), None);
        assert_eq!(resolve_package_name("node:fs"), None);
        assert_eq!(resolve_package_name("https://x.com/a"), None);
        assert_eq!(resolve_package_name(""), None);
    }

    #[test]
    fn extract_import_specifiers_variants() {
        let src = r#"
            import React from 'react';
            import { foo } from "@scope/lib/sub";
            import 'side-effect';
            export const x = 1; // export ... from
            export * from 're-exported';
            const y = require('lodash');
            const z = require("./relative");
            const dyn = import('dynamic-import');
            const node = require('node:path');
        "#;
        let specs = extract_import_specifiers(src);
        assert!(specs.contains(&"react".to_string()));
        assert!(specs.contains(&"@scope/lib/sub".to_string()));
        assert!(specs.contains(&"side-effect".to_string()));
        assert!(specs.contains(&"re-exported".to_string()));
        assert!(specs.contains(&"lodash".to_string()));
        assert!(specs.contains(&"dynamic-import".to_string()));
        // 相对/内置在原始提取中仍会出现；它们在 resolve_package_name 阶段被过滤。
        assert!(specs.contains(&"./relative".to_string()));
        assert!(specs.contains(&"node:path".to_string()));
    }

    #[test]
    fn analyze_finds_unused_runtime_dep() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(
            &root.join("package.json"),
            r#"{"name":"app","dependencies":{"used":"1.0.0","unused":"2.0.0"},"devDependencies":{"eslint":"8.0.0"}}"#,
        );
        // used 被引用；unused 未被引用；eslint（dev）未被引用
        touch(&root.join("src/index.ts"), "import x from 'used';\nconst a = require('used/util');");
        // 制造一个 node_modules，避免被当成不存在
        touch(&root.join("node_modules/used/index.js"), "x");
        touch(&root.join("node_modules/unused/index.js"), "x");

        let report = analyze_deps(root).unwrap();
        assert_eq!(report.eco, Eco::Node);
        assert_eq!(report.declared_count, 3);
        assert_eq!(report.used_count, 1);

        let names: Vec<&str> = report.unused.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"unused"), "unused 应被列为未使用: {names:?}");
        assert!(!names.contains(&"used"), "used 不应在未使用列表: {names:?}");

        // 运行期未使用 → High；开发期未使用 → Review
        let unused_entry = report.unused.iter().find(|d| d.name == "unused").unwrap();
        assert_eq!(unused_entry.kind, DepKind::Runtime);
        assert_eq!(unused_entry.confidence, DepConfidence::High);
        let eslint_entry = report.unused.iter().find(|d| d.name == "eslint").unwrap();
        assert_eq!(eslint_entry.kind, DepKind::Dev);
        assert_eq!(eslint_entry.confidence, DepConfidence::Review);
    }

    #[test]
    fn analyze_reports_extraneous() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(
            &root.join("package.json"),
            r#"{"name":"app","dependencies":{"used":"1.0.0"}}"#,
        );
        touch(&root.join("src/index.ts"), "import x from 'used';");
        touch(&root.join("node_modules/used/index.js"), "x");
        // 不在 package.json 中的顶层目录 → 多余
        touch(&root.join("node_modules/orphan/index.js"), "x");

        let report = analyze_deps(root).unwrap();
        let ex: Vec<&str> = report.extraneous.iter().map(|d| d.name.as_str()).collect();
        assert!(ex.contains(&"orphan"), "orphan 应被列为多余: {ex:?}");
        assert!(!ex.contains(&"used"));
    }

    #[test]
    fn prune_removes_from_manifest_and_trashes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let manifest = r#"{"name":"app","dependencies":{"keep":"1.0.0","drop":"2.0.0"}}"#;
        touch(&root.join("package.json"), manifest);
        touch(&root.join("node_modules/drop/index.js"), "big");
        touch(&root.join("node_modules/keep/index.js"), "small");

        let rep = prune_deps(root, &["drop".to_string()], false).unwrap();
        assert_eq!(rep.removed, vec!["drop".to_string()]);
        assert!(rep.backup_path.is_some());
        // freed_bytes 取决于 trash 是否可用（无桌面/回收站环境下可能失败并记录到 failed）。
        // 只要"释放了空间"或"记录了该依赖的移入回收站失败"都算合格——核心保证是清单已改写。
        assert!(
            rep.freed_bytes > 0 || rep.failed.iter().any(|(n, _)| n == "drop"),
            "drop 应被释放或记录为失败（环境无回收站）: {rep:?}"
        );

        // package.json 已不含 drop
        let after = std::fs::read_to_string(root.join("package.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert!(json["dependencies"].get("drop").is_none());
        assert!(json["dependencies"].get("keep").is_some());
        // 备份存在
        assert!(root.join("package.json.sweep.bak").exists());
        // 注意：trash 在 CI/无桌面环境下可能失败，这里不强制断言目录已被移走
    }

    #[test]
    fn prune_dry_run_touches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(
            &root.join("package.json"),
            r#"{"name":"app","dependencies":{"drop":"2.0.0"}}"#,
        );
        touch(&root.join("node_modules/drop/index.js"), "x");

        let rep = prune_deps(root, &["drop".to_string()], true).unwrap();
        assert!(rep.dry_run);
        assert!(rep.backup_path.is_none());
        // 原文件未变
        let after = std::fs::read_to_string(root.join("package.json")).unwrap();
        assert!(after.contains("drop"));
        assert!(root.join("node_modules/drop").exists());
    }

    #[test]
    fn analyze_rejects_non_node() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(analyze_deps(tmp.path()).is_err());
    }

    #[test]
    fn detect_pm_identifies_lockfiles() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert_eq!(detect_pm(root), PmKind::Unknown);

        touch(&root.join("package-lock.json"), "{}");
        assert_eq!(detect_pm(root), PmKind::Npm);
        fs::remove_file(root.join("package-lock.json")).unwrap();

        touch(&root.join("yarn.lock"), "{}");
        assert_eq!(detect_pm(root), PmKind::Yarn);
        fs::remove_file(root.join("yarn.lock")).unwrap();

        touch(&root.join("pnpm-lock.yaml"), "{}");
        assert_eq!(detect_pm(root), PmKind::Pnpm);
    }

    #[test]
    fn migrate_rejects_unknown_pm() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(
            &root.join("package.json"),
            r#"{"name":"app","dependencies":{}}"#,
        );
        // 没有任何 npm/yarn 锁文件 → 不安全，dry-run 也直接拒绝
        assert!(migrate_to_pnpm(root, true).is_err());
    }

    #[test]
    fn migrate_dry_run_reports_action() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(
            &root.join("package.json"),
            r#"{"name":"app","dependencies":{}}"#,
        );
        touch(&root.join("package-lock.json"), "{}");
        let rep = migrate_to_pnpm(root, true).unwrap();
        assert!(rep.dry_run);
        assert_eq!(rep.from_pm, PmKind::Npm);
        assert!(!rep.reinstalled);
        assert!(rep.error.is_none());
    }

    #[test]
    fn migrate_rejects_already_pnpm() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(
            &root.join("package.json"),
            r#"{"name":"app","dependencies":{}}"#,
        );
        touch(&root.join("pnpm-lock.yaml"), "{}");
        assert!(migrate_to_pnpm(root, true).is_err());
    }

    #[test]
    fn migrate_runs_when_pnpm_available() {
        // 仅当系统真的装了 pnpm 才真实执行迁移，否则跳过（避免 CI 误报）
        if !pnpm_available() {
            eprintln!("跳过：未检测到 pnpm（请先 `npm i -g pnpm`）");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // 构造最小 npm 项目（含一份合法的 lockfileVersion 3 锁文件，便于 pnpm import 转换）
        touch(
            &root.join("package.json"),
            r#"{"name":"app","dependencies":{"lodash":"^4.17.21"}}"#,
        );
        touch(
            &root.join("package-lock.json"),
            r#"{
  "name": "app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": { "name": "app", "version": "1.0.0", "dependencies": { "lodash": "^4.17.21" } },
    "node_modules/lodash": {
      "version": "4.17.21",
      "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
      "integrity": "sha512-v2kDEe57lecTulaDIuNTPy3Ry4gLGJ6Z1O3vE1krgXZNrsQ+LFTGHVxVjcXPs17LhbZVGedAJv8XZ1tvj5FvSg=="
    }
  }
}"#,
        );
        let rep = migrate_to_pnpm(root, false).unwrap();
        // pnpm-lock.yaml 由 `pnpm import` 离线生成，是迁移的确定性产物——必须存在
        assert!(
            root.join("pnpm-lock.yaml").is_file(),
            "应生成 pnpm-lock.yaml（来自 pnpm import）: {rep:?}"
        );
        // reinstalled 取决于网络：`pnpm install` 下载失败（沙箱无网）属环境限制，
        // 此时用户可用回收站里的旧 node_modules 恢复后手动 `pnpm install`，不判失败。
        if !rep.reinstalled {
            eprintln!(
                "提示：pnpm install 未成功（可能无网络），但 import 已生成 pnpm-lock.yaml: {:?}",
                rep.error
            );
        }
    }

    /// 系统是否安装 pnpm（用于条件跳过）。
    fn pnpm_available() -> bool {
        std::process::Command::new("pnpm")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
