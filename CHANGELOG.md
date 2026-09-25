# 更新日志

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)。 notable 变更记录于此。

## [Unreleased]

## [0.3.0] — 2026-09-24

可取消性与实时反馈版：所有长任务不再只能干等。

### 新增
- **批量删除可取消**：删除中可随时取消（UI 按钮 / Esc），中断后剩余项保留勾选便于续删
- **迁移实时输出 + 可取消**：pnpm/uv 迁移的子进程输出逐行转发到 GUI（显示尾部日志），并可中途取消；`pnpm import` 先行的安全顺序不变
- **决策日志（opt-in）**：`sweep clean/deps --log-decisions` 或 GUI 确认框开关，把删除/裁剪决策追加到 `~/.dev-sweeper/decisions.jsonl`（默认关闭）——为后续智能判定攒可审计数据

### 改进
- `scan:size` 事件 50ms 批量应用（同 id 后到覆盖 + 收尾冲刷）：数千产物时不再逐条全量重渲染
- git 陈旧度查询按 `project_dir` 建缓存：同项目多个产物只 spawn 一次 git，扫描明显变快
- 取消槽位竞态收尾：扫描收尾用 `Arc::ptr_eq` 比对后再清槽，晚到的旧一轮不再误清新一轮的取消标志（delete/migrate 槽同防护）

## [0.2.1] — 2026-09-23

一轮系统性代码审查（约 1 万行全量 + 依赖源码交叉核对）后的安全与正确性修复。

### 修复（安全类）
- **pnpm 迁移顺序**：此前先回收旧锁文件再跑 `pnpm import`，而 import 的唯一职责就是读旧锁文件——真实环境必然失败且项目已被拆散（CI 无回收站侥幸通过）。改为 import 成功产出 `pnpm-lock.yaml` 后再回收，import 失败则项目原封不动。
- **「排除路径」保护覆盖归档流程**：归档是破坏面最大的操作（整个项目源码进回收站），此前完全绕过 excludes。现在发现时过滤 + 核心层硬拒绝（纵深防御），CLI `archives`/`archive` 子命令新增 `--exclude` 旗标。
- **排除前缀在 Windows/macOS 上忽略大小写**：手工输入 `d:/important` 此护栏不住实际路径 `D:/Important`，保护静默失效。
- **`prune_deps` 安全对齐 delete.rs 不变量**：`remove` 参数在后端重新对照最新"未使用"清单（清单外一律拒绝，前端任意载荷点不出 react）；node_modules 内 symlink/junction 拒绝处置（防 pnpm workspace 误伤）。
- **归档打包/还原原子化**：tar 打包不再解引用 symlink/junction（默认 follow 会把链接目标拉平拍进归档）；`.part` 临时文件 + 原子改名；还原前 gzip 完整性预检；staging 中转，失败不留半解压目录挡住重试；`restored_bytes` 统计口径修正。
- **uv 缓存清理指向 `cache` 子目录**，不再整删 `%LOCALAPPDATA%\uv`；Windows 默认值兜底解析修正。

### 修复（正确性类）
- **依赖"未使用"误报修补**：npm scripts 里以命令行使用的包（prisma/husky 等）、CSS/Sass 的 `@plugin`/`@import`/`@use` 引用、postcss/babel/jest/tailwind/eslint 配置文件里的插件名、`.mts`/`.cts` 扩展名，现在都算"使用"；检测到动态 require/import 时 Runtime 未使用从 High 降级为 Review。
- **monorepo 陈旧度失真**：git 最后活跃取"最后触碰该子目录"的提交（pathspec），不再被父仓库任意提交污染。
- **扫描事件代际号**：取消旧扫描后其迟到事件不再按 id 碰撞污染新扫描结果。
- **重写 package.json 不再重排键序**（serde_json `preserve_order`）。
- **缓存面板**：清理失败的项不再从列表消失，保留勾选便于重试。
- **依赖分析并发防护**：请求序号守卫，旧响应不再覆盖新响应。

### 修复（CLI）
- Windows 传统控制台（GBK 代码页）中文与 emoji 乱码：启动时切 UTF-8 代码页。
- 失败路径退出码统一为 1（裁剪/迁移/归档/还原失败、缓存未知 id、部分项失败），脚本与 CI 可感知。
- `clean --no-size` 如实报告清理项数并注明跳过大小统计（此前报"共释放 0 B"）。

### 测试
- 核心测试 49 → 57（新增归档排除/损坏归档拒绝、依赖 scripts/CSS/mts/配置文件/白名单拒绝等回归测试）。

## [0.2.0] — 2026-09-01

### 新增
- Archive：沉睡项目发现（`sweep archives`）+ 压缩归档/还原（`sweep archive`/`restore`），GUI `ArchivePanel`
- Caches：全局缓存发现与清理（npm/pip/cargo/maven/gradle/go/uv/pnpm），GUI `CachesPanel`
- Deps：未使用依赖分析与瘦身（`sweep deps`），GUI `DepsPanel`
- UvMigrate：pip/poetry → uv 迁移（`sweep uv-migrate`）

### 改进
- 新增 `regex` 依赖，规则匹配增强
- DESIGN.md / README.md 同步更新（排除路径、风险等级说明）

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
