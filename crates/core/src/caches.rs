//! 全局依赖缓存清理（通用层）。
//!
//! 与「逐项目产物」(node_modules / target / .venv) 相对，这里针对**各语言包管理器
//! 的全局共享缓存**（~/.npm、~/.cargo/registry、~/.m2、~/.gradle、GOPATH/pkg/mod、
//! pip/uv 缓存、pnpm 全局存储等）。这些缓存：
//! - 全部可由 lockfile / 命令重新生成（清了只是重下，不丢源码）；
//! - 随开发者职业生涯累积，是「其他生态依赖」占空间的主要来源；
//! - 且默认不在项目内，普通 `rm node_modules` 根本碰不到。
//!
//! 安全模型与全工具一致：**一律移入回收站（trash），绝不 remove_dir_all**，可逆。

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::scan::dir_size;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CacheEco {
    Node,
    Rust,
    Java,
    Go,
    Python,
    Pnpm,
    Unknown,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub id: String,
    pub eco: CacheEco,
    pub label: String,
    /// 实际解析到的绝对路径（已确认存在）
    pub path: String,
    pub size_bytes: u64,
    /// 如何重新生成（给用户看的恢复提示）
    pub regen_hint: String,
    /// "safe" = 纯下载缓存，随时重下；"notice" = 清了会损失去重/需重装
    pub risk: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CachePurgeReport {
    pub id: String,
    pub path: String,
    pub freed_bytes: u64,
    pub reinstallable: bool,
    pub error: Option<String>,
    pub dry_run: bool,
}

/// 一个「逻辑缓存」候选：可能跨平台有多个路径提示，取第一个存在的。
struct Candidate {
    id: &'static str,
    eco: CacheEco,
    label: &'static str,
    hints: &'static [&'static str],
    regen_hint: &'static str,
    risk: &'static str,
}

/// 候选白名单。占位符 `${KEY}` 在 `env_table` 中会被解析（含各包管理器默认值）。
fn candidates() -> Vec<Candidate> {
    vec![
        Candidate {
            id: "node-npm-cache",
            eco: CacheEco::Node,
            label: "npm 缓存",
            hints: &["${LOCALAPPDATA}/npm-cache", "${APPDATA}/npm-cache", "${HOME}/.npm"],
            regen_hint: "重新安装依赖时自动下载",
            risk: "safe",
        },
        Candidate {
            id: "rust-cargo-registry",
            eco: CacheEco::Rust,
            label: "Cargo 注册表缓存",
            hints: &["${CARGO_HOME}/registry"],
            regen_hint: "cargo build 时重新下载 crate 源码",
            risk: "safe",
        },
        Candidate {
            id: "rust-cargo-git",
            eco: CacheEco::Rust,
            label: "Cargo git 依赖缓存",
            hints: &["${CARGO_HOME}/git"],
            regen_hint: "cargo build 时重新拉取 git 依赖",
            risk: "safe",
        },
        Candidate {
            id: "java-maven",
            eco: CacheEco::Java,
            label: "Maven 本地仓库",
            hints: &["${M2_REPO}", "${HOME}/.m2/repository", "${USERPROFILE}/.m2/repository"],
            regen_hint: "mvn 构建时按 pom 重新下载",
            risk: "safe",
        },
        Candidate {
            id: "java-gradle",
            eco: CacheEco::Java,
            label: "Gradle 缓存",
            hints: &["${HOME}/.gradle/caches", "${USERPROFILE}/.gradle/caches"],
            regen_hint: "gradle 构建时重新下载",
            risk: "safe",
        },
        Candidate {
            id: "go-mod",
            eco: CacheEco::Go,
            label: "Go module 缓存",
            hints: &["${GOPATH}/pkg/mod"],
            regen_hint: "go build 时重新下载模块",
            risk: "safe",
        },
        Candidate {
            id: "python-pip",
            eco: CacheEco::Python,
            label: "pip 缓存",
            hints: &[
                "${XDG_CACHE_HOME}/pip",
                "${LOCALAPPDATA}/pip",
                "${HOME}/.cache/pip",
            ],
            regen_hint: "pip install 时自动重下",
            risk: "safe",
        },
        Candidate {
            id: "python-uv",
            eco: CacheEco::Python,
            label: "uv 缓存（内容寻址）",
            hints: &["${UV_CACHE_DIR}", "${HOME}/.cache/uv", "${LOCALAPPDATA}/uv"],
            regen_hint: "uv 会重建全局缓存；清后首次安装较慢，但跨项目去重会恢复",
            risk: "notice",
        },
        Candidate {
            id: "pnpm-store",
            eco: CacheEco::Pnpm,
            label: "pnpm 全局存储",
            hints: &[
                "${PNPM_STORE_DIR}",
                "${HOME}/.pnpm-store",
                "${LOCALAPPDATA}/pnpm/store",
            ],
            regen_hint: "pnpm install 时重建；清后跨项目去重失效，需重新下载",
            risk: "notice",
        },
    ]
}

fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default()
}

fn var_or(key: &str, default: String) -> String {
    std::env::var(key).unwrap_or(default)
}

/// 解析用的环境变量表：真实 env 优先，缺失时给各包管理器的「默认位置」。
fn env_table(home: &str) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    m.insert("HOME".into(), home.into());
    m.insert("USERPROFILE".into(), home.into());
    m.insert(
        "LOCALAPPDATA".into(),
        var_or("LOCALAPPDATA", format!("{home}/AppData/Local")),
    );
    m.insert(
        "APPDATA".into(),
        var_or("APPDATA", format!("{home}/AppData/Roaming")),
    );
    m.insert(
        "XDG_CACHE_HOME".into(),
        var_or("XDG_CACHE_HOME", format!("{home}/.cache")),
    );
    m.insert(
        "CARGO_HOME".into(),
        var_or("CARGO_HOME", format!("{home}/.cargo")),
    );
    m.insert("GOPATH".into(), var_or("GOPATH", format!("{home}/go")));
    m.insert(
        "M2_REPO".into(),
        var_or("M2_REPO", format!("{home}/.m2/repository")),
    );
    m.insert(
        "UV_CACHE_DIR".into(),
        var_or("UV_CACHE_DIR", format!("{home}/.cache/uv")),
    );
    m.insert(
        "PNPM_STORE_DIR".into(),
        var_or(
            "PNPM_STORE_DIR",
            if cfg!(windows) {
                format!("{home}/AppData/Local/pnpm/store")
            } else {
                format!("{home}/.pnpm-store")
            },
        ),
    );
    m
}

/// 把 `${KEY}` 与 `~` 解析成真实路径。缺失的 KEY 保留原样（后续因不存在而被跳过）。
fn resolve_path(raw: &str, home: &str, envs: &HashMap<String, String>) -> String {
    let mut s = raw.replace('~', home);
    for (k, v) in envs {
        s = s.replace(&format!("${{{k}}}"), v);
    }
    s
}

/// 发现本机存在的全局缓存，按白名单顺序返回（每个逻辑缓存取第一个存在的路径）。
pub fn discover_global_caches() -> Vec<CacheEntry> {
    let home = home_dir();
    let envs = env_table(&home);
    let mut out = Vec::new();
    for c in candidates() {
        for hint in c.hints {
            let p = resolve_path(hint, &home, &envs);
            let path = Path::new(&p);
            if path.exists() {
                let size = dir_size(path);
                out.push(CacheEntry {
                    id: c.id.into(),
                    eco: c.eco,
                    label: c.label.into(),
                    path: p,
                    size_bytes: size,
                    regen_hint: c.regen_hint.into(),
                    risk: c.risk.into(),
                });
                break; // 第一个存在的提示路径即代表该逻辑缓存
            }
        }
    }
    out
}

/// 清理单个路径：先算大小（置于 trash 之前），再移入回收站。dry_run 只报告。
fn purge_path(path: &Path, dry_run: bool) -> CachePurgeReport {
    let p = path.to_string_lossy().into_owned();
    let size = dir_size(path);
    if dry_run {
        return CachePurgeReport {
            id: String::new(),
            path: p,
            freed_bytes: size,
            reinstallable: true,
            error: None,
            dry_run: true,
        };
    }
    match trash::delete(path) {
        Ok(()) => CachePurgeReport {
            id: String::new(),
            path: p,
            freed_bytes: size,
            reinstallable: true,
            error: None,
            dry_run: false,
        },
        Err(e) => CachePurgeReport {
            id: String::new(),
            path: p,
            freed_bytes: 0,
            reinstallable: true,
            error: Some(e.to_string()),
            dry_run: false,
        },
    }
}

/// 按 id 清理某个全局缓存；找不到返回 Err。
pub fn purge_cache(id: &str, dry_run: bool) -> Result<CachePurgeReport, String> {
    let entry = discover_global_caches()
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("未找到全局缓存: {id}"))?;
    let mut rep = purge_path(Path::new(&entry.path), dry_run);
    rep.id = id.to_string();
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_expands_tilde_and_vars() {
        let mut envs = HashMap::new();
        envs.insert("HOME".into(), "/home/me".into());
        envs.insert("CARGO_HOME".into(), "/home/me/.cargo".into());
        assert_eq!(
            resolve_path("${CARGO_HOME}/registry", "/home/me", &envs),
            "/home/me/.cargo/registry"
        );
        assert_eq!(resolve_path("~/foo", "/home/me", &envs), "/home/me/foo");
        // 缺失 KEY 保留占位符（后续不会 exists → 被跳过）
        assert_eq!(
            resolve_path("${NOPE}/x", "/home/me", &envs),
            "${NOPE}/x"
        );
    }

    #[test]
    fn discover_returns_unique_ids() {
        let v = discover_global_caches();
        let mut seen = std::collections::HashSet::new();
        for e in &v {
            assert!(seen.insert(e.id.clone()), "重复 id: {}", e.id);
            assert!(!e.path.is_empty());
        }
    }

    #[test]
    fn purge_path_dry_run_touches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), "1234").unwrap();
        let rep = purge_path(tmp.path(), true);
        assert!(rep.dry_run);
        assert_eq!(rep.freed_bytes, 4);
        assert!(rep.error.is_none());
        assert!(tmp.path().join("a").exists(), "dry-run 不应删除任何文件");
    }

    #[test]
    fn purge_path_real_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), "1234").unwrap();
        let rep = purge_path(tmp.path(), false);
        assert!(!rep.dry_run);
        // 沙箱可能无回收站导致 trash 失败；要么删成功、要么报错，但不应 panic
        assert!(rep.freed_bytes > 0 || rep.error.is_some());
    }
}
