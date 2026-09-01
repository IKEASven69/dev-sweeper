//! Python 生态迁移：pip / poetry / pip-tools → uv。
//!
//! uv 使用**全局内容寻址缓存**（`~/.cache/uv`），并把每个 venv 的 site-packages 以
//! 硬链接方式指向缓存里的 wheel，从而跨项目去重——与 pnpm 的"内容寻址全局存储 +
//! 硬链接 node_modules"是同一套省盘逻辑，对应 dev-sweeper "不删也能瘦" 在 Python 侧的杠杆。
//!
//! 安全边界（比 pnpm 更保守，因为 uv 迁移边界更多）：
//! - **前置守卫**：未检测到 uv 时直接返回 Err，绝不移动任何文件（破坏性操作仅在
//!   确认工具可用后才发生）。
//! - 真实迁移时，先把旧 `.venv` 移入回收站（可恢复），再 `uv venv` + 安装。
//! - 安装失败不影响已被回收站保护的旧 `.venv`，用户可恢复后手动处理。

use std::path::Path;

use serde::Serialize;

/// Python 包管理器类型（按清单/锁文件识别）。
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PyPmKind {
    /// 传统 pip + venv（requirements.txt / setup.py / 无锁的 pyproject.toml）
    Pip,
    /// 已是 uv 管理（存在 uv.lock）
    Uv,
    /// poetry（存在 poetry.lock）
    Poetry,
    /// pip-tools（requirements.in + requirements.txt，带 --hash）
    PipTools,
    /// 无法识别
    Unknown,
}

/// 按清单/锁文件识别当前 Python 包管理器。
pub fn detect_pypm(dir: &Path) -> PyPmKind {
    if dir.join("uv.lock").is_file() {
        PyPmKind::Uv
    } else if dir.join("poetry.lock").is_file() {
        PyPmKind::Poetry
    } else if dir.join("requirements.in").is_file() && dir.join("requirements.txt").is_file() {
        PyPmKind::PipTools
    } else if dir.join("requirements.txt").is_file()
        || dir.join("pyproject.toml").is_file()
        || dir.join("setup.py").is_file()
    {
        PyPmKind::Pip
    } else {
        PyPmKind::Unknown
    }
}

/// 系统是否安装 uv（用于条件跳过 / 前置守卫）。
pub fn uv_available() -> bool {
    std::process::Command::new("uv")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 把 pip / poetry / pip-tools 项目迁移到 uv 的执行报告（字段与 `MigrateReport` 对齐）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MigratePyReport {
    /// 迁移前的包管理器（来自清单识别）。
    pub from_pm: PyPmKind,
    /// 旧 .venv 移入回收站释放的体积（字节）。
    pub freed_bytes: u64,
    /// 被移入回收站的旧 .venv 原路径（用于提示用户"已回收站可恢复"）。
    pub backup_path: Option<String>,
    /// 是否成功执行 uv venv + 安装。
    pub reinstalled: bool,
    /// 迁移过程错误（若有）。
    pub error: Option<String>,
    /// 是否仅演练。
    pub dry_run: bool,
}

/// 判断目录是否像 Python 项目（有任一 Python 清单）。
fn looks_like_python(dir: &Path) -> bool {
    dir.join("requirements.txt").is_file()
        || dir.join("pyproject.toml").is_file()
        || dir.join("setup.py").is_file()
        || dir.join("setup.cfg").is_file()
}

/// 把 pip / poetry / pip-tools 项目迁移到 uv。
///
/// 步骤：
/// 1. 仅当检测到 Python 项目（requirements.txt / pyproject.toml / setup.py）才允许迁移；
///    Unknown 拒绝；已是 uv（存在 uv.lock）直接拒绝。
/// 2. **前置守卫**：uv 未安装则直接 Err，不碰任何文件（uv 迁移边界多，优先安全）。
/// 3. dry_run 只报告"会做什么"，不移动不安装。
/// 4. 非空跑：先把旧 `.venv` 移入回收站以立即释放磁盘（可恢复），再 `uv venv` 建硬链接
///    venv，并按清单安装：
///      - pyproject.toml → `uv sync`（生成 uv.lock）
///      - requirements.txt → `uv pip install -r requirements.txt`
///      - setup.py（无 pyproject）→ `uv pip install -e .`
///
/// 安全边界：与 `migrate_to_pnpm` 一致，删除一律走回收站（`trash::delete`），永不
/// `remove_dir_all`；安装失败不影响已被回收站保护的旧 `.venv`，用户可随时恢复。
pub fn migrate_to_uv(dir: &Path, dry_run: bool) -> Result<MigratePyReport, String> {
    if !looks_like_python(dir) {
        return Err(
            "未检测到 Python 项目（需 requirements.txt / pyproject.toml / setup.py）".into(),
        );
    }
    let from_pm = detect_pypm(dir);
    if from_pm == PyPmKind::Uv {
        return Err("项目已是 uv 管理（存在 uv.lock），无需迁移".into());
    }
    if from_pm == PyPmKind::Unknown {
        return Err("未能识别 Python 包管理器".into());
    }

    // 前置守卫：未装 uv 直接拒绝，零破坏（uv 迁移边界多，安全优先）。
    if !uv_available() {
        return Err(
            "未检测到 uv（安装：https://docs.astral.sh/uv/getting-started/installation/）。迁移不执行任何修改。".into(),
        );
    }

    if dry_run {
        return Ok(MigratePyReport {
            from_pm,
            freed_bytes: 0,
            backup_path: None,
            reinstalled: false,
            error: None,
            dry_run,
        });
    }

    // 1) 备份旧 .venv 到回收站，立即释放磁盘（可恢复）
    let venv = dir.join(".venv");
    let mut freed_bytes = 0u64;
    let mut backup_path: Option<String> = None;
    if venv.is_dir() {
        freed_bytes += crate::scan::dir_size(&venv);
        match trash::delete(&venv) {
            Ok(()) => backup_path = Some(venv.to_string_lossy().into_owned()),
            Err(e) => {
                return Ok(MigratePyReport {
                    from_pm,
                    freed_bytes: 0,
                    backup_path: None,
                    reinstalled: false,
                    error: Some(format!("无法将旧 .venv 移入回收站: {e}")),
                    dry_run,
                });
            }
        }
    }

    // 2) uv venv + 按清单安装
    let mut result: Result<(), String> = run_uv(&["venv"], dir);
    if result.is_ok() {
        if dir.join("pyproject.toml").is_file() {
            result = run_uv(&["sync"], dir);
        } else if dir.join("requirements.txt").is_file() {
            result = run_uv(&["pip", "install", "-r", "requirements.txt"], dir);
        } else if dir.join("setup.py").is_file() {
            result = run_uv(&["pip", "install", "-e", "."], dir);
        }
    }
    let reinstalled = result.is_ok();
    let error = result.err();

    Ok(MigratePyReport {
        from_pm,
        freed_bytes,
        backup_path,
        reinstalled,
        error,
        dry_run,
    })
}

/// 运行 uv 子命令。
fn run_uv(args: &[&str], dir: &Path) -> Result<(), String> {
    let full: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    match std::process::Command::new("uv").args(&full).current_dir(dir).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("`uv {}` 退出码 {}", full.join(" "), s)),
        Err(e) => Err(format!("无法执行 `uv`: {e}")),
    }
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
    fn detect_pypm_basics() {
        let t1 = tempfile::tempdir().unwrap();
        touch(&t1.path().join("requirements.txt"), "requests==2.31.0\n");
        assert_eq!(detect_pypm(t1.path()), PyPmKind::Pip);

        let t2 = tempfile::tempdir().unwrap();
        touch(&t2.path().join("uv.lock"), "version: 1\n");
        assert_eq!(detect_pypm(t2.path()), PyPmKind::Uv);

        let t3 = tempfile::tempdir().unwrap();
        touch(&t3.path().join("poetry.lock"), "x");
        assert_eq!(detect_pypm(t3.path()), PyPmKind::Poetry);

        let t4 = tempfile::tempdir().unwrap();
        touch(&t4.path().join("requirements.in"), "requests\n");
        touch(&t4.path().join("requirements.txt"), "requests==2.31.0\n");
        assert_eq!(detect_pypm(t4.path()), PyPmKind::PipTools);
    }

    #[test]
    fn migrate_rejects_non_python() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("README.md"), "hi");
        assert!(migrate_to_uv(tmp.path(), true).is_err());
    }

    #[test]
    fn migrate_rejects_already_uv() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("pyproject.toml"), "[project]\nname = \"x\"\n");
        touch(&tmp.path().join("uv.lock"), "version: 1\n");
        assert!(migrate_to_uv(tmp.path(), true).is_err());
    }

    #[test]
    fn migrate_dry_run_reports_action() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("requirements.txt"), "requests==2.31.0\n");
        let rep = migrate_to_uv(tmp.path(), true).unwrap();
        assert!(rep.dry_run);
        assert_eq!(rep.from_pm, PyPmKind::Pip);
        assert!(!rep.reinstalled);
    }

    // 前置守卫仅在「系统未装 uv」时验证（零破坏）。uv 已安装时，真实迁移需联网下载
    // 依赖，受限网络下会挂起，故跳过真实路径——仅验证 dry-run 不联网、不创建 .venv。
    #[test]
    fn migrate_guards_without_uv() {
        if uv_available() {
            eprintln!("跳过真实迁移：uv 已安装，但联网安装受限于沙箱网络，避免挂起");
            let tmp = tempfile::tempdir().unwrap();
            touch(&tmp.path().join("requirements.txt"), "requests==2.31.0\n");
            let rep = migrate_to_uv(tmp.path(), true).unwrap();
            assert!(rep.dry_run);
            assert!(!tmp.path().join(".venv").exists(), "dry-run 不应创建 .venv");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("requirements.txt"), "requests==2.31.0\n");
        let res = migrate_to_uv(tmp.path(), false);
        assert!(res.is_err(), "未装 uv 时必须拒绝迁移");
        assert!(
            !tmp.path().join(".venv").exists(),
            "不应创建 .venv（零破坏）"
        );
    }
}
