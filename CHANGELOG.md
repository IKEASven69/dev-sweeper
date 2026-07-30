# 更新日志

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)。 notable 变更记录于此。

## [Unreleased]

## [0.1.0] — 2026-07

首个可用版本。多生态开发产物扫描清理工具，删除一律移入回收站（可恢复）。

### 核心
- **17 生态**扫描：node / rust / maven / gradle / python(venv+cache) / cmake / dotnet / composer / unity / unreal / godot / swift / zig / elixir / cocoapods / web-dist。规则表驱动，加生态 = 加一行。
- **marker 双闸防误删**：目录名命中 + 标记文件确认（`package.json`/`Cargo.toml`/`pyvenv.cfg`…）。新增 `ParentHasSuffix` 处理 `.uproject`/`.csproj` 后缀标记。
- **回收站删除**：一律 `trash`，无永久删除路径，不可关闭。删除前 `validate_marker` 复查 + 拒绝 symlink（TOCTOU 防御）。
- **dry-run**：CLI `sweep clean -n` + GUI 预演按钮。
- **风险等级**：🟢 Safe（构建产物秒级恢复）/ 🟡 Notice（依赖环境重装慢）。

### 精度与安全
- **git-aware aging**：`last_active_ms` 融合 mtime + git 最后 commit 时间（取 max），非 git 项目回退纯 mtime。
- **排除路径**：`--exclude` 前缀保护"永不清理"的目录（CLI + GUI，GUI 持久化到 localStorage）。

### 形态
- **桌面 GUI**（Tauri 2 + React 19）：图标列表布局、扫描取消、进度计数、4 统计卡、键盘快捷键（Ctrl+A / Esc）、排除芯片、risk 徽标。
- **CLI**（`sweep`）：`scan` / `clean`，支持 `--rules` `--stale-days` `--exclude` `--no-size` `--json` `--dry-run`。

### 工程化
- **CI**：三平台（ubuntu/windows/macos）cargo test + tsc，零 warning。
- **Release CI**：tag 触发三平台自动打包（NSIS/dmg/deb/AppImage/rpm），首发 unsigned（macOS ad-hoc 签名）。
- **23 单测** + 跨平台大小写不敏感匹配 + 可取消扫描。
