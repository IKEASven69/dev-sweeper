# dev-sweeper

[![CI](https://github.com/IKEASven69/dev-sweeper/actions/workflows/ci.yml/badge.svg)](https://github.com/IKEASven69/dev-sweeper/actions/workflows/ci.yml)
[![Release](https://github.com/IKEASven69/dev-sweeper/actions/workflows/release.yml/badge.svg)](https://github.com/IKEASven69/dev-sweeper/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

多生态开发产物扫描清理工具：一次扫描找出所有 `node_modules`、Rust/Maven 的 `target`、Gradle 的 `build`、Python 的 `.venv` / `__pycache__`，按大小和陈旧度排序，批量**移入回收站**。还提供「依赖瘦身」——精准找出 `package.json` 里声明了却从未用到的依赖并移除，而不是整包删 `node_modules`；**pnpm 迁移**——把 npm/yarn 项目迁移到 pnpm 内容寻址存储，跨项目去重省盘；以及 **清全局缓存**——集中清理各语言包管理器的全局共享缓存（npm / pip / cargo / maven / gradle / go / uv / pnpm），它们不在项目内、普通清理碰不到、清了只是重新下载；以及 **压缩归档**——把沉睡项目整体压成 .tar.gz 移出工作区，源码不丢、随时可还原（"不删也能瘦"）。

> 定位：唯一一个跨所有语言、按陈旧度排序、只碰可重建产物且全程可逆的开发者空间回收器。删除一律进回收站，绝不 `rm -rf`。

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

### 依赖瘦身（"重构依赖"）

除了删产物，还能精准裁剪**没用到的依赖**——找出 `package.json` 里声明了但源码从未引用的依赖并移除，而不是整包删 `node_modules`：

```
sweep deps D:\Projects\my-app              # 列出未使用/多余的依赖（含置信度）
sweep deps D:\Projects\my-app --apply      # 确认后移除未使用依赖（package.json 先备份，对应目录进回收站）
sweep deps D:\Projects\my-app --json       # 脚本用
```

- 分析只读；`--apply` 会把 `package.json` 备份为 `package.json.sweep.bak`，并把对应 `node_modules/<pkg>` 目录移入回收站（可恢复）。
- 运行期依赖（几乎可确定没用）标 **高**，开发依赖（可能仅由 CLI/配置使用）标 **需复核**，请先确认。
- `node_modules` 中不在清单里的目录会提示，建议用 `npm prune` / `pnpm prune` 让包管理器权威判定。
- 桌面 GUI 另有「依赖瘦身」标签页，可视化勾选后一键重构。

### pnpm 迁移（省盘）

npm/yarn 项目每个都存一份扁平 `node_modules`，大量项目时极费磁盘。pnpm 用**内容寻址全局存储**，同一份包跨项目只存一份。可把现有 npm/yarn 项目迁移到 pnpm 瘦身：

```
sweep migrate D:\Projects\my-app          # 交互确认后迁移（旧 node_modules 进回收站，再 pnpm import + install）
sweep migrate D:\Projects\my-app -n       # 预演：只报告会做什么
sweep migrate D:\Projects\my-app -y       # 跳过确认
```

- 仅当存在 `package-lock.json` / `yarn.lock` 才允许迁移；已是 pnpm（`pnpm-lock.yaml`）或无锁文件则拒绝。
- 迁移先把旧 `node_modules` 与旧锁文件移入回收站（可恢复），再 `pnpm import` + `pnpm install` 重建。
- 若 `pnpm install` 失败（如离线），旧 `node_modules` 仍可在回收站恢复，再手动 `pnpm install` 收尾——不存在"删了装不回"的死角。
- 桌面 GUI 的「依赖瘦身」标签页在识别到 npm/yarn 时会显示「迁移到 pnpm」入口。

### uv 迁移（Python 版 pnpm，省盘）

与 pnpm 完全对称：`uv` 用**全局内容寻址缓存**（`~/.cache/uv`），并把每个 venv 的 `site-packages` 以**硬链接**指向缓存里的 wheel——传统 `python -m venv` + `pip install` 会在每个项目里物理拷贝一份包，迁移到 uv 后跨项目去重。需先安装 `uv`（https://docs.astral.sh/uv/）：

```
sweep uv-migrate D:\Projects\my-py        # 交互确认后迁移（旧 .venv 进回收站，再 uv venv + 安装）
sweep uv-migrate D:\Projects\my-py -n     # 预演：只报告会做什么
sweep uv-migrate D:\Projects\my-py -y     # 跳过确认
```

- 支持 `requirements.txt`（`uv pip install -r`）、`pyproject.toml`（`uv sync` 生成 `uv.lock`）、`setup.py`（`uv pip install -e .`）。
- **无 `uv` 时直接拒绝且不碰任何文件**（安全前置守卫）；已是 uv（`uv.lock`）或非 Python 目录则拒绝。
- 迁移先把旧 `.venv` 移入回收站（可恢复），再重建 uv 管理的 venv——不存在"删了装不回"的死角。

### 清全局缓存（通用省盘）

除了项目内的产物，各语言包管理器的**全局共享缓存**也默默吃盘，且不在项目内、普通清理碰不到。它们清了只是重新下载，不丢任何源码——是"其他生态依赖怎么省盘"的通用答案（pnpm 只管 Node，而 Maven/Gradle/Go/Rust 的依赖本来就全局共享，Python 有 `uv`）：

```
sweep caches                            # 列出本机全部全局缓存（含大小/风险/路径）
sweep caches --id python-pip           # 只清理指定缓存
sweep caches --apply                    # 清理全部已发现
sweep caches --apply -n                 # 预演：只报告会释放多少
sweep caches --json                     # 脚本用
```

- 发现阶段只列存在的缓存、不删；清理只动白名单内的全局缓存目录（npm/pip/cargo/maven/gradle/go/uv/pnpm），**一律移入回收站**可恢复。
- 风险分两级：纯下载缓存（npm/pip/cargo/gradle/maven/go）标 **安全**；会损失跨项目去重或需重装的（uv / pnpm store）标 **注意**，清了首次安装会慢一点但去重会恢复。
- 桌面 GUI 另有「全局缓存」标签页，自动发现、勾选后一键清理（含预演与确认弹窗）。

### 压缩归档（不删也能瘦）

活的项目的源码不能删，但可以压成可恢复的归档，需要时一键解回——把"睡得最久的项目"优先移出工作区，释放即时空间又不丢东西：

```
sweep archives D:\Projects --stale-days 180   # 发现沉睡项目（按最后活跃升序）
sweep archive D:\Projects\old-app             # 压成 .tar.gz（默认 ~/dev-archives），原项目进回收站
sweep archive D:\Projects\old-app -n          # 预演：只报告会释放多少
sweep restore D:\Projects\old-app@20260101.tar.gz --dest D:\Projects   # 解回
```

- 发现阶段扫描项目根下一级子目录、按"最后活跃"升序，沉睡最久的排最前；`--stale-days` 只列超过 N 天未动的项目。
- 归档后原项目**移入回收站**（可恢复），归档文件本身也是一份副本；清空回收站即真正释放磁盘——与"删 node_modules"行为统一。
- 桌面 GUI 另有「压缩归档」标签页，可视化发现沉睡项目、勾选后一键归档，并管理归档库（列出 + 还原），均含预演与确认弹窗。

## 结构

```
crates/core   # 扫描/删除核心（walkdir + rayon + trash），规则表驱动，单元测试在此
crates/cli    # sweep 命令（clap）
src-tauri     # Tauri 壳：scan/delete_artifacts/analyze_deps/prune_deps/migrate_to_pnpm/migrate_to_uv/discover_caches/purge_cache + 压缩归档 4 命令（discover_archivable/archive_project/list_archives/restore_archive）+ 事件流
src           # React 前端（Tailwind v4），含 ArchivePanel.tsx（压缩归档标签页）
```

## 安全设计

- 删除前校验路径末段必须是已知产物目录名（`crates/core/src/delete.rs`）
- 一律 `trash` 移入回收站，不做永久删除
- 扫描不 follow symlink（pnpm 软链不重复计数），跳过 `.git`，无权限目录静默跳过
- 命中的产物目录不再深入（node_modules 内嵌套的不重复记录）
