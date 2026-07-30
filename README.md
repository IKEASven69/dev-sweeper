# dev-sweeper

[![CI](https://github.com/IKEASven69/dev-sweeper/actions/workflows/ci.yml/badge.svg)](https://github.com/IKEASven69/dev-sweeper/actions/workflows/ci.yml)
[![Release](https://github.com/IKEASven69/dev-sweeper/actions/workflows/release.yml/badge.svg)](https://github.com/IKEASven69/dev-sweeper/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

多生态开发产物扫描清理工具：一次扫描找出所有 `node_modules`、Rust/Maven 的 `target`、Gradle 的 `build`、Python 的 `.venv` / `__pycache__`，按大小和陈旧度排序，批量**移入回收站**。

桌面 GUI（Tauri 2 + React 19）+ CLI（`sweep`），共享同一个 Rust 扫描核心。

## 为什么不用 npkill / kondo

| | npkill | kondo | **dev-sweeper** |
|---|---|---|---|
| 形态 | 终端 TUI | CLI（+老旧 GUI） | 现代桌面 GUI + CLI |
| 多生态并扫 | ✗（一次一种目录名） | ✓ | ✓ |
| 删除方式 | **永久删除** | **永久删除**（自述 "rm -rf with a prompt"） | **回收站，可恢复** |
| 陈旧项目可视化 | ✗ | `--older` 标志 | 大小 × 最后活跃排序 + 一键全选陈旧项 |
| 再生提示 | ✗ | ✗ | 每条附 `pnpm install` / `cargo build` 等提示 |

核心差异：**怕删错的人也敢用**。删除一律进回收站，误删可恢复；每个产物目录附"怎么再生成"的提示。

## 清理规则

目录名命中 + 标记文件确认（防误删同名目录）：

| 生态 | 目录 | 确认条件 |
|---|---|---|
| node | `node_modules` | 父目录含 `package.json` |
| rust | `target` | 父目录含 `Cargo.toml` |
| maven | `target` | 父目录含 `pom.xml` |
| gradle | `build`、`.gradle` | 父目录含 `build.gradle(.kts)` / `settings.gradle` |
| python-venv | `.venv`、`venv`、`env` | 目录内含 `pyvenv.cfg` |
| python-cache | `__pycache__`、`.pytest_cache` | — |
| web-dist（默认关） | `.next`、`dist` | 父目录含 `package.json` |

加生态 = 在 `crates/core/src/rules.rs` 的规则表加一行。

## 使用

### GUI

```
pnpm install
pnpm tauri dev      # 开发
pnpm tauri build    # 打包（NSIS 安装包，位于 target/release/bundle/nsis/）
```

选择或粘贴目录 → 扫描 → 条目实时流入、大小渐进填充 → 勾选（或"全选陈旧项"）→ 移入回收站。

### CLI

```
cargo build --release -p dev-sweeper-cli   # 产出 target/release/sweep.exe

sweep scan D:\Projects                     # 列出产物，按大小降序
sweep scan D:\Projects --rules node,rust --stale-days 90 --json
sweep scan D:\Projects --no-size           # 快速列出，不算大小
sweep scan D:\Projects --exclude D:\important  # 保护目录不扫描
sweep clean D:\Projects --stale-days 180   # 确认后移入回收站
sweep clean D:\Projects -n                 # 预演：只校验不删除
```

## 结构

```
crates/core   # 扫描/删除核心（walkdir + rayon + trash），规则表驱动，单元测试在此
crates/cli    # sweep 命令（clap）
src-tauri     # Tauri 壳：scan/delete_artifacts 命令 + scan:found/scan:size 事件流
src           # React 前端（Tailwind v4）
```

## 安全设计

- 删除前校验路径末段必须是已知产物目录名（`crates/core/src/delete.rs`）
- 一律 `trash` 移入回收站，不做永久删除
- 扫描不 follow symlink（pnpm 软链不重复计数），跳过 `.git`，无权限目录静默跳过
- 命中的产物目录不再深入（node_modules 内嵌套的不重复记录）
