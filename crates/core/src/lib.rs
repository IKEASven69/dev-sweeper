pub mod delete;
pub mod rules;
pub mod scan;

pub use delete::{delete_to_trash, validate_artifact_path};
pub use rules::{select_rules, CleanRule, Marker, RULES};
pub use scan::{compute_sizes, dir_size, scan_artifacts, Artifact};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn touch(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn scan_all(root: &Path) -> Vec<Artifact> {
        let rules = select_rules(&[]);
        scan_artifacts(root, &rules, |_| {})
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
    fn compute_sizes_fills_and_reports() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        touch(&tmp.path().join("app/node_modules/a.js"), "1234");

        let mut found = scan_all(tmp.path());
        let reported = std::sync::Mutex::new(Vec::new());
        compute_sizes(&mut found, |id, size| reported.lock().unwrap().push((id, size)));

        assert_eq!(found[0].size_bytes, Some(4));
        assert_eq!(*reported.lock().unwrap(), vec![(0, 4)]);
    }

    #[test]
    fn delete_rejects_non_artifact_path() {
        assert!(validate_artifact_path(Path::new("C:/Users/me/Documents")).is_err());
        assert!(validate_artifact_path(Path::new("C:/proj/node_modules")).is_ok());
        assert!(validate_artifact_path(Path::new("C:/proj/target")).is_ok());
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
}
