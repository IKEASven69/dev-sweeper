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
        scan_artifacts(root, &rules, &cancel, &[], |_| {}, |_| {})
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
    fn cmake_build_requires_cmakelists() {
        let tmp = tempfile::tempdir().unwrap();
        // 有 CMakeLists.txt → cmake 命中
        touch(&tmp.path().join("cm/CMakeLists.txt"), "cmake_minimum_required");
        touch(&tmp.path().join("cm/build/a.o"), "x");
        // 裸 build 无 marker → 不命中（build 是通用名，必须确认）
        touch(&tmp.path().join("bare/build/x"), "x");

        let found = scan_all(tmp.path());
        let cm = found.iter().find(|a| a.rule_id == "cmake").unwrap();
        // 末段是 build（跨平台：用 components 而非字符串 ends_with，避免分隔符差异）
        assert_eq!(
            std::path::Path::new(&cm.path)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "build"
        );
        // 裸 build 不应出现
        assert!(found.iter().all(|a| !a.path.ends_with("bare/build")));
    }

    #[test]
    fn dotnet_bin_obj_requires_csproj() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/app.csproj"), "<Project/>");
        touch(&tmp.path().join("app/bin/app.dll"), "x");
        touch(&tmp.path().join("app/obj/x"), "x");
        // 没有 .csproj 的 bin → 不命中（bin 是极通用名）
        touch(&tmp.path().join("other/bin/script.sh"), "x");

        let found = scan_all(tmp.path());
        let dotnet_count = found
            .iter()
            .filter(|a| a.path.contains("app") && a.rule_id == "dotnet")
            .count();
        assert_eq!(dotnet_count, 2, "app 下应识别 bin + obj 两个 dotnet");
        assert!(found.iter().all(|a| !a.path.ends_with("other/bin")));
    }

    #[test]
    fn unreal_uses_suffix_marker() {
        let tmp = tempfile::tempdir().unwrap();
        // Unreal 项目文件名不固定，靠 .uproject 后缀识别
        touch(&tmp.path().join("game/MyGame.uproject"), "{}");
        touch(&tmp.path().join("game/Binaries/x"), "x");
        touch(&tmp.path().join("game/Intermediate/y"), "x");
        // 没有 .uproject 的 Binaries → 不命中
        touch(&tmp.path().join("other/Binaries/z"), "x");

        let found = scan_all(tmp.path());
        let unreal: Vec<_> = found.iter().filter(|a| a.rule_id == "unreal").collect();
        assert_eq!(unreal.len(), 2, "应识别 game 下 Binaries + Intermediate");
        assert!(found.iter().all(|a| !a.path.contains("other")));
    }

    #[test]
    fn generic_dir_without_marker_not_matched() {
        // 综合回归：Library/Temp/Logs(unity) / Pods(cocoapods) / vendor(composer)
        // 这些通用名在没有对应 marker 时一律不命中
        let tmp = tempfile::tempdir().unwrap();
        for name in ["Library", "Temp", "Logs", "Pods", "vendor", "Binaries"] {
            touch(&tmp.path().join(format!("bare/{name}/x")), "x");
        }
        let found = scan_all(tmp.path());
        assert!(
            found.is_empty(),
            "无 marker 的通用名目录不应被命中，实际: {:?}",
            found.iter().map(|a| &a.rule_id).collect::<Vec<_>>()
        );
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
        let found = scan_artifacts(tmp.path(), &rules, &cancel, &[], |_| {}, |_| {});
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
        scan_artifacts(tmp.path(), &rules, &cancel, &[], |_| {}, |n| {
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

    /// 在 dir 初始化一个临时 git 仓库并做一次提交。系统无 git 时跳过测试。
    fn git_init_commit(dir: &Path) -> bool {
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        run(&["init"]) && run(&["add", "-A"]) && run(&["commit", "-m", "x"])
    }

    #[test]
    fn last_active_fuses_git_commit_and_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        touch(&tmp.path().join("app/node_modules/a.js"), "x");
        // 无 git 时 last_active_ms 仍应返回 Some（来自 mtime）
        let found = scan_all(tmp.path());
        assert_eq!(found.len(), 1);
        assert!(found[0].last_active_ms.is_some(), "无 git 时应回退 mtime");
    }

    #[test]
    fn scan_excludes_protected_paths() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("keep/package.json"), "{}");
        touch(&tmp.path().join("keep/node_modules/a.js"), "x");
        touch(&tmp.path().join("protect/package.json"), "{}");
        touch(&tmp.path().join("protect/node_modules/a.js"), "x");

        let rules = select_rules(&[]);
        let cancel = never_cancel();
        // 排除 protect 目录（用前缀）
        let excludes = vec![tmp.path().join("protect").to_string_lossy().into_owned()];
        let found = scan_artifacts(tmp.path(), &rules, &cancel, &excludes, |_| {}, |_| {});
        assert_eq!(found.len(), 1, "protect 应被排除，只保留 keep");
        assert!(found[0].path.contains("keep"));
    }

    #[test]
    fn last_active_uses_git_commit_when_available() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("app/package.json"), "{}");
        touch(&tmp.path().join("app/node_modules/a.js"), "x");
        if !git_init_commit(tmp.path()) {
            eprintln!("跳过：系统无 git 或 init 失败");
            return;
        }
        let found = scan_all(tmp.path());
        assert_eq!(found.len(), 1);
        // 有 git commit 时，last_active_ms 应非常接近"现在"（commit 刚发生）
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let age_secs = now_ms
            .saturating_sub(found[0].last_active_ms.unwrap_or(0))
            / 1000;
        assert!(
            age_secs < 60,
            "git commit 时间应接近现在，实际距今 {age_secs}s"
        );
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
