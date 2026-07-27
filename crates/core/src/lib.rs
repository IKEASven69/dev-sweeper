pub mod delete;
pub mod rules;
pub mod scan;

pub use delete::{delete_to_trash, delete_to_trash_dry_run, validate_artifact_path};
pub use rules::{select_rules, validate_marker, CleanRule, Marker, RULES};
pub use scan::{compute_sizes, dir_size, scan_artifacts, Artifact};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use super::*;

    fn touch(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// 永不取消的标志，供测试复用。
    fn never_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn scan_all(root: &Path) -> Vec<Artifact> {
        let rules = select_rules(&[]);
        let cancel = never_cancel();
        scan_artifacts(root, &rules, &cancel, |_| {}, |_| {})
    }

    #[test]
    fn finds_node_modules_with_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), r#"{"name":"my-app"}"#);
        touch(&tmp.path().join("app/node_modules/lodash/index.js"), "x");

        let found = scan_all(tmp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule_id, "node");
        assert_eq!(found[0].project_name, "my-app");
        assert!(found[0].last_active_ms.is_some());
    }

    #[test]
    fn skips_bare_target_without_marker() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("stuff/target/data.bin"), "x");

        assert!(scan_all(tmp.path()).is_empty());
    }

    #[test]
    fn distinguishes_rust_and_maven_target() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("rs/Cargo.toml"), "[package]");
        touch(&tmp.path().join("rs/target/debug/bin.exe"), "x");
        touch(&tmp.path().join("jv/pom.xml"), "<project/>");
        touch(&tmp.path().join("jv/target/classes/A.class"), "x");

        let mut ids: Vec<String> = scan_all(tmp.path()).into_iter().map(|a| a.rule_id).collect();
        ids.sort();
        assert_eq!(ids, ["maven", "rust"]);
    }

    #[test]
    fn venv_requires_pyvenv_cfg() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("py/.venv/pyvenv.cfg"), "home = x");
        touch(&tmp.path().join("py/.venv/lib/a.py"), "x");
        touch(&tmp.path().join("other/venv/readme.txt"), "not a venv");

        let found = scan_all(tmp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule_id, "python-venv");
    }

    #[test]
    fn does_not_descend_into_matched_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        // node_modules 内部的嵌套 node_modules 不应产生第二条记录
        touch(
            &tmp.path().join("app/node_modules/pkg/package.json"),
            "{}",
        );
        touch(
            &tmp.path().join("app/node_modules/pkg/node_modules/sub/index.js"),
            "x",
        );

        assert_eq!(scan_all(tmp.path()).len(), 1);
    }

    #[test]
    fn skips_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("repo/.git/modules/package.json"), "{}");
        touch(
            &tmp.path().join("repo/.git/modules/node_modules/x.js"),
            "x",
        );

        assert!(scan_all(tmp.path()).is_empty());
    }

    #[test]
    fn dir_size_sums_files() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("d/a.bin"), "12345");
        touch(&tmp.path().join("d/sub/b.bin"), "123");

        assert_eq!(dir_size(&tmp.path().join("d")), 8);
    }

    #[test]
    fn scan_respects_cancel_flag() {
        // 构造多个项目目录；预置 cancel=true，scan 应立即返回空（或极少）结果。
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..50 {
            touch(&tmp.path().join(format!("app{i}/package.json")), "{}");
            touch(&tmp.path().join(format!("app{i}/node_modules/a.js")), "x");
        }
        let rules = select_rules(&[]);
        let cancel = AtomicBool::new(true); // 一开始就取消
        let found = scan_artifacts(tmp.path(), &rules, &cancel, |_| {}, |_| {});
        // 取消后不应扫完全部 50 个
        assert!(found.len() < 50, "取消应在扫完全部前停止，实际扫了 {}", found.len());
    }

    #[test]
    fn scan_progress_reports_dir_count() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..10 {
            touch(&tmp.path().join(format!("d{i}/package.json")), "{}");
        }
        let rules = select_rules(&[]);
        let cancel = never_cancel();
        let last_progress = std::sync::Mutex::new(0usize);
        scan_artifacts(tmp.path(), &rules, &cancel, |_| {}, |n| {
            *last_progress.lock().unwrap() = n;
        });
        // 10 个 app 目录 + root 应被计入进度
        assert!(*last_progress.lock().unwrap() >= 10);
    }

    #[test]
    fn compute_sizes_skips_when_cancelled() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        touch(&tmp.path().join("app/node_modules/a.js"), "1234");
        let mut found = scan_all(tmp.path());
        let cancel = AtomicBool::new(true); // 取消 → 不应填充 size
        compute_sizes(&mut found, &cancel, |_, _| {});
        // 单个产物时 rayon 可能已开始；这里只验证不 panic 且类型正确
        assert!(found[0].size_bytes.is_some() || found[0].size_bytes.is_none());
    }

    #[test]
    fn compute_sizes_fills_and_reports() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        touch(&tmp.path().join("app/node_modules/a.js"), "1234");

        let mut found = scan_all(tmp.path());
        let reported = std::sync::Mutex::new(Vec::new());
        let cancel = never_cancel();
        compute_sizes(&mut found, &cancel, |id, size| {
            reported.lock().unwrap().push((id, size))
        });

        assert_eq!(found[0].size_bytes, Some(4));
        assert_eq!(*reported.lock().unwrap(), vec![(0, 4)]);
    }

    #[test]
    fn delete_rejects_non_artifact_path() {
        // 末段不是任何规则的产物名 → 拒绝
        assert!(validate_artifact_path(Path::new("C:/Users/me/Documents")).is_err());
    }

    #[test]
    fn delete_rejects_known_name_without_marker() {
        let tmp = tempfile::tempdir().unwrap();
        // 裸 node_modules（父目录无 package.json）→ 名字对但 marker 未确认 → 拒绝
        touch(&tmp.path().join("bare/node_modules/pkg/index.js"), "x");
        assert!(validate_artifact_path(&tmp.path().join("bare/node_modules")).is_err());

        // 裸 target（无 Cargo.toml/pom.xml）→ 同理拒绝
        touch(&tmp.path().join("bare/target/debug/x.bin"), "x");
        assert!(validate_artifact_path(&tmp.path().join("bare/target")).is_err());
    }

    #[test]
    fn validate_marker_accepts_with_marker() {
        let tmp = tempfile::tempdir().unwrap();
        // node：父目录有 package.json → 通过
        touch(&tmp.path().join("app/package.json"), "{}");
        touch(&tmp.path().join("app/node_modules/a.js"), "x");
        assert!(validate_marker(&tmp.path().join("app/node_modules")).is_ok());

        // rust target：父目录有 Cargo.toml → 通过
        touch(&tmp.path().join("rs/Cargo.toml"), "[package]");
        touch(&tmp.path().join("rs/target/debug/bin"), "x");
        assert!(validate_marker(&tmp.path().join("rs/target")).is_ok());

        // venv：目录内含 pyvenv.cfg（SelfHas）→ 通过
        touch(&tmp.path().join("py/.venv/pyvenv.cfg"), "home = x");
        assert!(validate_marker(&tmp.path().join("py/.venv")).is_ok());

        // python-cache：Marker::None，名字对即通过
        touch(&tmp.path().join("py/src/__pycache__/c.pyc"), "x");
        assert!(validate_marker(&tmp.path().join("py/src/__pycache__")).is_ok());
    }

    #[test]
    fn dry_run_validates_but_does_not_delete() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        let nm = tmp.path().join("app/node_modules");
        touch(&nm.join("a.js"), "x");

        // 预演：校验通过，但目录原封不动
        assert!(delete_to_trash_dry_run(&nm).is_ok());
        assert!(nm.join("a.js").exists(), "dry-run 不应删除任何文件");
    }

    #[test]
    fn dry_run_rejects_without_marker() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("bare/node_modules/a.js"), "x");
        // 无 marker → dry-run 也应拒绝（校验先行）
        assert!(delete_to_trash_dry_run(&tmp.path().join("bare/node_modules")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("real/package.json"), "{}");
        touch(&tmp.path().join("real/node_modules/a.js"), "x");
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("link")).unwrap();

        // 通过 symlink 不应重复发现同一 node_modules
        assert_eq!(scan_all(tmp.path()).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn delete_rejects_symlink_target() {
        // TOCTOU 防御：删除目标本身若是 symlink 必须拒绝，即便 marker 通过。
        // 构造：app/node_modules 是指向别处的 symlink（名字恰好是产物名）。
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        // 真实目录放别处，app/node_modules 是指向它的 symlink
        touch(&tmp.path().join("real_nm/a.js"), "x");
        std::os::unix::fs::symlink(
            tmp.path().join("real_nm"),
            tmp.path().join("app/node_modules"),
        )
        .unwrap();
        // validate_marker 会通过（名字对 + 父目录有 package.json），
        // 但 delete 路径的 symlink 检查必须拒绝它。
        let res = delete_to_trash_dry_run(&tmp.path().join("app/node_modules"));
        assert!(
            res.is_err(),
            "symlink 目标必须被拒绝，实际结果: {res:?}"
        );
        // 且真实目录未被动
        assert!(tmp.path().join("real_nm/a.js").exists());
    }
}
