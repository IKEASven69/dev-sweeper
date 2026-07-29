# dev-sweeper 设计文档

> 多生态开发产物扫描清理工具。扫描找出 `node_modules` / Rust `target` / Python `.venv` / Gradle `build` / `__pycache__` 等构建与依赖产物，按大小和陈旧度排序，**移入回收站**（可恢复），并为每个产物附"如何再生"的提示。
>
> 形态：桌面 GUI（Tauri 2 + React 19）+ CLI（`sweep`），共享同一个 Rust 扫描核心。

---

## 1. 背景与定位

### 1.1 问题

开发机器上累积大量可再生产物：一个中型前端项目的 `node_modules` 动辄数百 MB；Rust 的 `target`、Python 的 `.venv`、Gradle 的 `build` 同理。它们分散在数十个项目目录里，手动清理既繁琐又危险（误删源码）。

### 1.2 核心主张

**"怕删错的人也敢用。"**

- 删除一律进回收站，误删可恢复——不是 `rm -rf`。
- 每个产物目录都附"怎么再生成"的提示（`pnpm install` / `cargo build` / `python -m venv` …），降低"删了之后不会恢复"的心理负担。
- 标记文件确认（`package.json` / `Cargo.toml` / `pyvenv.cfg` …），防止误删同名目录。
- 现代跨平台桌面 GUI，而非终端 TUI 或老旧 GUI。

### 1.3 非目标

- **不做**永久删除（`rm -rf`）—— 即使高级用户也走回收站。
- **不做**全局缓存清理（`npm cache`、`pnpm store prune`、`cargo-cache`、`~/.gradle/caches`）—— 那是另一类工具的职责；dev-sweeper 聚焦"项目内可再生产物"。
- **不做**依赖分析（depcheck / knip 的"未使用依赖"）—— 只关心磁盘占用。
- **不做**隐私擦除 / 系统垃圾清理（BleachBit 的领地）。

---

## 2. 竞品分析

> 调研截至 2026-07。核心结论：**删除方式（回收站 vs 永久）+ 跨平台 GUI** 是两个最硬的差异化点。

### 2.1 直接竞品（多生态开发产物清理）

| 工具 | 形态 | 生态覆盖 | 删除方式 | 陈旧度 | 跨平台 | 再生提示 |
|---|---|---|---|---|---|---|
| **kondo** | CLI 交互 + Bevy GUI（重写中） | **25 类**（最广：Cargo/Node/Unity/Unreal/CMake/…) | **永久删除**（`remove_dir_all`，自述 "rm -rf"） | `--older` 过滤 | ✅ Win/Mac/Lin | ✗ |
| **npkill** | TUI | 发布版 node 居中；未发布 `main` 已扩到 17 profile | **永久删除**（`rm -rf` / `fs.rm`） | 仅排序，无 `--older` 过滤 | ✅ | ✗ |
| **ClearDisk** ⭐ | macOS 菜单栏原生 App | **63 缓存路径 + 23 项目类型**（最全：含 Xcode/AI 工具/Unity/Unreal） | **回收站** ✅ | 风险等级 🟢🟡🔴 | ✗ **仅 macOS** | ✅ 丰富 |
| **mac-cleanup** | Rust TUI | pip/Go/Cargo/npm/Gradle + Xcode + 系统 | 永久删除 | ✗ | ✗ macOS | ✗ |
| **dev-cleaner** | Bash/PS 脚本 | Xcode/Flutter/Gradle/npm/IDE | 永久删除 | ✗ | ✅ | ✗ |

### 2.2 相关工具（非直接竞品但界定边界）

- **单生态缓存清理**：`cargo-cache`（Rust，含项目 `target/`）、`npm cache clean`、`pnpm store prune` —— 权威但各管一摊，且多为全局缓存而非项目产物。
- **通用磁盘可视化**：Baobab（已移除删除）、QDirStat（**唯一**支持回收站 + 可脚本化清理动作）、WizTree（Win/MFT 极快）、SquirrelDisk（Tauri+React，但通用非 dev-aware）—— 找得到大目录，但"能否安全删"留给用户。
- **终端 du**：`ncdu`（可配 `--delete-command 'gio trash'` 走回收站，但无生态感知）、`dust`（只读）、`duf`（df 替代）。
- **系统自带**：macOS Storage Manager / Windows Storage Sense —— **均无开发产物感知**，Storage Sense 甚至定期清空回收站。
- **商业清理**：CleanMyMac / CleanMyPC —— 付费、闭源、dev 覆盖浅且不透明。
- **生态专用**：DevCleaner for Xcode（Xcode-only，**永久删除**）、`delete_derived_data`（Xcode-only，走回收站，**macOS-only**）。

### 2.3 差异化定位

逐项对照后，dev-sweeper 落在的空白：

> **跨平台（Win/Mac/Lin）+ 多生态 + 回收站 + 再生提示 + 现代 GUI** 的组合，目前**没有任何工具同时满足**。

- **vs kondo**（2.3k★，incumbent）：生态最广、架构好（lib/bin 分离），但 **6 年来永久删除**、aging 用整棵树最新 mtime（一个日志文件就能让陈旧项目"显得活跃"）。dev-sweeper 的回收站 + 更准的 aging 模型正中其软肋。
- **vs npkill**（9.4k★，最流行）：纯目录名匹配无 marker 确认（`target`/`build`/`obj` 易误报）、永久删除、无 age 过滤（issue #257 开放中）。未发布的 `main` 虽扩到 17 profile 但**浅**（不区分 Rust `target` 和 Java `target`）。dev-sweeper 的 marker 确认 + 默认回收站直接对应其用户已在 issue 里提的需求（#60 要 trash）。
- **vs ClearDisk**（**最接近的对手**）：产品主张几乎一致（多生态 + 回收站 + 再生提示 + 原生 GUI），其作者已验证了这个 thesis。**唯一软肋是 macOS-only**。dev-sweeper 的机会窗口：**做跨平台的 ClearDisk**——Win/Linux 开发者在这个品类里除了永久删除的脚本一无所有。

### 2.4 竞品启示录（将转化为路线图）

| 启示 | 来源 | dev-sweeper 对应动作 |
|---|---|---|
| 回收站是杀手锏 | npkill #60 / kondo 全局 / DevCleaner | ✅ 已有，**继续作为头号卖点** |
| marker 确认降误报 | npkill 纯名匹配的软肋 | ✅ 已有 `Marker` 设计 |
| aging 要准 | kondo 整树 mtime 的缺陷 | ⚠️ 现状用标记文件 mtime，已比 kondo 准；可引入 git last-commit |
| 生态广度 | ClearDisk 63+23、kondo 25 | ⚠️ 现状 7 类，需扩充（Unity/Unreal/CMake/.NET/Composer…） |
| 风险等级可视化 | ClearDisk 🟢🟡🔴 | ✅ 已加 `Risk::Safe/Notice`，CLI 列 + GUI 徽标 |
| dry-run | kondo `-n` / npkill `--dry-run` | 📋 待办 |
| 跨平台 GUI 是空白 | ClearDisk macOS-only | ✅ Tauri 天然跨平台，**核心壁垒** |

---

## 3. 系统架构

### 3.1 总览

```
┌─────────────────────────────────────────────────────────┐
│                   Cargo workspace                        │
│                                                          │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐ │
│  │ crates/core  │   │ crates/cli   │   │  src-tauri   │ │
│  │ (扫描/删除)  │◄──┤ (sweep 命令) │   │ (Tauri 壳)   │ │
│  │  规则表驱动  │   └──────────────┘   └──────┬───────┘ │
│  │  单测在此    │          ▲                  │ 命令+事件 │
│  └──────┬───────┘          │                  ▼         │
│         │        walkdir / rayon / trash     IPC        │
│         ▼                                    │          │
└─────────────────────────────────────────────────────────┘
                                              │
                              ┌───────────────▼──────────┐
                              │       src (React 19)      │
                              │   Tailwind v4 前端        │
                              └───────────────────────────┘
```

**三 crate 分层**（参照 kondo 的 lib/bin 分离最佳实践）：

| crate | 职责 | 依赖 |
|---|---|---|
| `dev-sweeper-core` | 扫描、大小计算、删除校验、规则表；**纯逻辑、无 IO 框架**、所有单测集中于此 | `walkdir` `rayon` `trash` `serde` |
| `dev-sweeper-cli` | `sweep` 命令（clap + comfy-table），薄壳 | `core` `clap` `comfy-table` |
| `dev-sweeper`（src-tauri） | Tauri 壳：`scan`/`delete_artifacts` 命令 + `scan:found`/`scan:size`/`delete:progress` 事件流 | `core` `tauri` + 插件 |

**核心约束**：所有业务逻辑在 `core`，CLI 与 Tauri 都只是"前端的 IO 适配"。这保证 GUI 与 CLI 行为一致，且 `core` 可被任意集成（未来 VS Code 插件、CI 脚本等）。

### 3.2 前后端契约（IPC）

**命令**（Tauri `invoke`）：

| 命令 | 入参 | 返回 | 副作用事件 |
|---|---|---|---|
| `scan` | `{ root: String, ruleIds: Vec<String> }` | `ScanSummary { count, totalBytes, elapsedMs }` | 每发现一个产物 emit `scan:found`；每个大小算完 emit `scan:size`；结束 emit `scan:done` |
| `delete_artifacts` | `{ paths: Vec<String> }` | `DeleteReport { deleted, failed }` | 每删一个 emit `delete:progress` |

**事件流**（`listen`）—— 这是"扫描条目实时流入、大小渐进填充"体验的关键：

```
scan:found  (Artifact)   ── 发现即推送，UI 立刻显示一行（大小显示 …）
scan:size   {id, size}   ── rayon 并行算完一个回填一个，进度条渐进增长
scan:done   (ScanSummary) ── 扫描+大小计算全部结束
delete:progress {done, total, path, ok, error?}
```

扫描跑在 `spawn_blocking` 里（`crates/core` 的同步 `walkdir` + `rayon`），通过 `AppHandle::emit` 把进度推给前端——避免阻塞 Tauri 的异步运行时。

---

## 4. 核心模块设计

### 4.1 规则表（`crates/core/src/rules.rs`）—— 加生态 = 加一行

这是整个工具可扩展性的支点。设计目标：**新增一个生态只需加一行 `CleanRule`，零代码分支**。

```rust
pub enum Marker {
    ParentHas(&'static [&'static str]), // 父目录含任一标记文件（package.json / Cargo.toml）
    SelfHas(&'static str),              // 目录自身含标记（venv 的 pyvenv.cfg）
    None,                               // 无需确认（__pycache__）
}

pub struct CleanRule {
    pub id: &'static str,               // "node" / "rust" / ...
    pub dir_names: &'static [&'static str], // ["node_modules"] / ["build", ".gradle"]
    pub marker: Marker,                 // 防误删确认条件
    pub regen_hint: &'static str,       // "npm install / pnpm install"
    pub risk: Risk,                     // 🟢 Safe（构建产物）/ 🟡 Notice（依赖/环境）
    pub default_on: bool,
    // TODO: pub weight: u8,             // 同名目录冲突时的优先级（见 4.1.2）
}
```

**为什么 marker 必要**：`target` 既是 Rust 又是 Maven 产物；任意目录都可能叫 `build`/`env`/`dist`。纯名匹配（npkill 的做法）在多生态下误报率高。marker 确认把"这是真产物"的把握从"名字对"提到"名字对 + 上下文对"。

**现状规则（17 类，default_on 标 ✱）**：

| id | dir_names | marker | regen_hint | 默认 |
|---|---|---|---|---|
| node | `node_modules` | ParentHas `[package.json]` | `npm install / pnpm install` | ✱ |
| rust | `target` | ParentHas `[Cargo.toml]` | `cargo build` | ✱ |
| maven | `target` | ParentHas `[pom.xml]` | `mvn package` | ✱ |
| gradle | `build`, `.gradle` | ParentHas `[build.gradle(.kts), settings.gradle(.kts)]` | `gradle build` | ✱ |
| python-venv | `.venv`, `venv`, `env` | SelfHas `pyvenv.cfg` | `python -m venv / uv sync` | ✱ |
| python-cache | `__pycache__`, `.pytest_cache` | None | `运行时自动再生` | ✱ |
| cmake | `build`, `cmake-build-*` | ParentHas `[CMakeLists.txt]` | `cmake --build` | ✱ |
| dotnet | `bin`, `obj` | ParentHasSuffix `[.csproj/.fsproj/.vbproj]` | `dotnet build` | ✱ |
| composer | `vendor` | ParentHas `[composer.json]` | `composer install` | ✱ |
| unity | `Library`, `Temp`, `Obj`, `Logs`, `MemoryCaptures` | ParentHas `[Assembly-CSharp.csproj]` | Unity 打开自动再生 | ✱ |
| unreal | `Binaries`, `Intermediate`, `DerivedDataCache`, `Saved` | ParentHasSuffix `[.uproject]` | Unreal 打开自动再生 | ✱ |
| godot | `.godot` | ParentHas `[project.godot]` | Godot 打开自动再生 | ✱ |
| swift | `.build`, `.swiftpm` | ParentHas `[Package.swift]` | `swift build` | ✱ |
| zig | `zig-cache`, `.zig-cache`, `zig-out` | ParentHas `[build.zig]` | `zig build` | ✱ |
| elixir | `_build`, `.elixir-ls` | ParentHas `[mix.exs]` | `mix compile` | ✱ |
| cocoapods | `Pods` | ParentHas `[Podfile]` | `pod install` | ✱ |
| web-dist | `.next`, `dist` | ParentHas `[package.json]` | `npm run build` | 关 |

> **Marker 新增 `ParentHasSuffix`**：处理文件名不固定的标记（Unreal 的 `*.uproject`、.NET 的 `*.csproj`）——父目录含指定**后缀**的文件即确认。通用目录名（`bin`/`obj`/`build`/`vendor`/`Library`/`Temp`）一律要求 marker 确认，杜绝误伤同名目录。

> **风险等级 `Risk`**（衡量"删后重生成成本"，回收站已兜底"能否恢复"）：
> - 🟢 **Safe**：纯构建输出，重新 build 秒级恢复（rust/maven/gradle/cmake/dotnet target/build、python-cache、unity/unreal/godot/swift/zig/elixir cache、web-dist）。
> - 🟡 **Notice**：依赖或环境，重装可能数分钟且需网络（node_modules、python-venv、composer vendor、cocoapods Pods）。
> - Artifact 带上 `risk` 字段序列化给前端；CLI 表格有"风险"列；GUI 列表显示徽标（🟢 易恢复 / 🟡 重装较慢）+ hover 提示。

#### 4.1.1 同名目录冲突处理

`target` 被 rust 和 maven 共用。`select_rules` 按 `RULES` 表内顺序 `find` 第一条 marker 命中的规则（`scan.rs::match_rule`）。

- **风险**：顺序隐式决定优先级。若一个 Maven 项目误含 `Cargo.toml`，会被判为 rust。
- **现状可接受**：两规则的 marker 互斥（`Cargo.toml` vs `pom.xml`），实践中不冲突。
- **演进**：引入显式 `weight` 字段或"全量 marker 检查后选最佳匹配"。

#### 4.1.2 规则筛选（`select_rules`）

- 传空 `ids` → 返回所有 `default_on` 的规则（默认体验：开箱即扫常见生态，`web-dist` 这类有争议的默认关）。
- 传非空 `ids` → 精确匹配（未知 id 静默忽略，容错）。
- `known_dir_names()`：所有规则覆盖的目录名并集，供删除前校验。

### 4.2 扫描（`crates/core/src/scan.rs`）

#### 4.2.1 单遍遍历算法

```rust
pub fn scan_artifacts(
    root: &Path,
    rules: &[&'static CleanRule],
    on_found: impl FnMut(&Artifact),   // 流式回调
) -> Vec<Artifact>
```

**关键决策**：

1. **命中即 `skip_current_dir`**：`node_modules` 内嵌套的 `node_modules` 不重复记录（避免一个项目被记 N 次）。单测 `does_not_descend_into_matched_dirs` 守住。
   - **取舍代价**：命中产物后不再深入，因此 `node_modules` 内若嵌套了**异生态**真项目（如 native addon 里的 Rust crate，其 `target`），会被漏掉。这是有意的——优先保证"不重复计数"的简洁语义，且这种嵌套真项目极罕见。
2. **跳过 `.git`**：版本控制目录不进，既省时间又避免 `.git/modules` 里的产物被误算。
3. **不 follow symlink**：pnpm 的软链不重复计数；防 symlink 环。
4. **遍历错误静默跳过**：无权限目录等 `Err(_) => continue`，扫描不会因一个不可读目录而中断。
5. **目录名匹配平台感知**（见 `rules.rs::dir_name_matches`）：Windows / macOS 文件系统大小写不敏感，故 `NODE_MODULES`、`Target`、`Build` 也识别；Linux 保持精确匹配（大小写不同即不同目录）。
6. **可取消**：`scan_artifacts` 接收 `&AtomicBool`，每次取下一个条目前检查；`compute_sizes` 的 rayon 任务开头也检查。GUI 的 `cancel_scan` 命令置位后扫描尽快收尾，返回已发现的部分结果（`ScanSummary.cancelled = true`）。
7. **进度回调**：每扫约 256 个目录调一次 `on_progress(scanned_dirs)`，GUI emit `scan:progress` 事件驱动进度条。

**为什么不预排序目录**：`walkdir` 流式产出，`on_found` 即时回调——前端能"边扫边显示"，体验远好于"扫完一次性出"。

#### 4.2.2 Artifact 数据模型

```rust
pub struct Artifact {
    pub id: u32,                  // = 在 found 数组中的下标，前端据此回填 size
    pub rule_id: String,
    pub path: String,
    pub project_dir: String,
    pub project_name: String,     // node 规则读 package.json 的 name，否则取目录名
    pub size_bytes: Option<u64>,  // None = 尚未计算（前端显示 …）
    pub last_active_ms: Option<u64>, // 见 4.2.4
    pub regen_hint: String,
}
```

`id = 下标` 这个设计让 `scan:size` 事件能 O(1) 定位回填目标行。

#### 4.2.3 大小计算（`compute_sizes`）—— 与扫描解耦

```rust
pub fn compute_sizes(artifacts: &mut [Artifact], on_sized: impl Fn(u32, u64) + Sync)
```

- **扫描结束后**才统一算大小，用 `rayon` 的 `par_iter_mut` **并行**。
- `dir_size` 用第二次 `walkdir`（follow_links(false)）只累加文件 `metadata().len()`。
- **为什么扫描时不算**：扫描是单线程流式（要尽快把"发现"推出去），大小计算是 CPU/IO 密集可并行——两阶段分离让各自的关注点清晰，且大小能"算完一个推一个"，UI 进度条平滑增长。

#### 4.2.4 陈旧度（`last_active_ms`）—— 比 kondo 更准

```rust
// 取项目标记文件 + src/ 目录 + project_dir 自身 的 mtime 最大值
const CANDIDATES: &[&str] = &[
    "package.json", "Cargo.toml", "pom.xml",
    "build.gradle", "build.gradle.kts", "settings.gradle",
    "pyproject.toml", "requirements.txt", "src",
];
```

- **vs kondo**：kondo 取整棵项目树（含产物目录）的最新 mtime——一个被触碰的日志/临时文件就让陈旧项目"显得活跃"。dev-sweeper 只看**源码侧标记文件**的 mtime，更接近"最后一次真正开发"的语义。
- **局限**：标记文件 mtime 仍可能被 `touch` 或工具改写而不代表真实开发。
- **演进**：可选引入 git last-commit 时间（`git log -1 --format=%ct`），但需权衡扫描成本与"非 git 项目"的覆盖。

### 4.3 删除（`crates/core/src/delete.rs`）—— 安全双闸

```rust
pub fn validate_artifact_path(path: &Path) -> Result<(), String>  // 闸一
pub fn delete_to_trash(path: &Path) -> Result<(), String>          // 闸二（先校验再 trash）
```

**闸一：路径末段必须是已知产物名**（`known_dir_names().contains(name)`）。防止调用方传任意路径。

**闸二：一律 `trash::delete`**，永不 `remove_dir_all`。这是与 kondo/npkill 的根本区别，也是头号卖点。

#### 4.3.1 删除前 marker 重校验（已实现）

删除前用 `validate_marker(path)`：取路径末段 → 反查规则（`rule_for_dir_name`）→ 重跑该规则的 marker 检查。与扫描时 `scan::match_rule` 走相同判定逻辑，使 `core` 公共 API 自洽——即使外部绕过 scan 直接调 `delete_to_trash`，也会拒绝"名字对、无 marker"的目录。

- `validate_marker` 返回 `Result<&CleanRule, String>`：成功时带回命中的规则（供调用方附加信息），失败时返回可读的拒绝原因（含缺失的 marker 描述）。
- 预演函数 `delete_to_trash_dry_run`：同样先过 `validate_marker` + 存在性检查，但不调用 `trash::delete`。
- 单测覆盖：`validate_marker_accepts_with_marker`（node/rust/venv/cache 四类正向）、`delete_rejects_known_name_without_marker`（裸 node_modules / 裸 target 被拒）、`dry_run_validates_but_does_not_delete`、`dry_run_rejects_without_marker`。

### 4.4 CLI（`crates/cli/src/main.rs`）

```
sweep scan <path> [--rules node,rust] [--stale-days 90] [--json]
sweep clean <path> [...] [--stale-days 180] [-y]
```

- `scan`：表格输出（comfy-table），按大小降序；`--json` 供脚本；`--stale-days` 过滤陈旧项。
- `clean`：先打印表格 → 默认 `confirm()` 交互确认 → 逐个 `delete_to_trash`，统计释放量与失败数，提示"可在回收站恢复"。
- `scan_and_size` 复用 core 的 `scan_artifacts` + `compute_sizes`，与 GUI 走完全相同的代码路径。

### 4.5 Tauri 壳（`src-tauri/src/lib.rs`）

两个 `#[tauri::command]`，都在 `spawn_blocking` 里跑同步 core 逻辑：

- `scan`：`scan_artifacts` 的 `on_found` → `app.emit("scan:found", a)`；`compute_sizes` 的 `on_sized` → `emit("scan:size", ...)`。
- `delete_artifacts`：循环 `delete_to_trash`，每步 `emit("delete:progress", ...)`，最终返回 `DeleteReport`。

能力声明（`capabilities/default.json`）：`core:default` + `opener:allow-reveal-item-in-dir`（"在资源管理器打开"按钮）+ `dialog:default`（目录选择器）。最小权限。

### 4.6 前端（`src/App.tsx`）

单文件 App，状态机驱动：

```
[输入 root] → scan → [artifacts 流式填充 + size 渐进] → [勾选/排序/过滤] → 确认弹窗 → delete → toast
```

**体验要点**：
- **流式 UI**：`listen("scan:found")` append、`listen("scan:size")` 原地回填——扫描中就能看到结果涌入。
- **三统计卡**：可回收空间 / 陈旧项目（>N 天未动）/ 已选中——一眼看清"删了能省多少、哪些最该删、当前选了多少"。
- **大小条形图**：每行用生态色画占比条，视觉直击"谁最占地方"。
- **"全选陈旧项"**：一键勾选所有超阈值的——降低决策成本。
- **确认弹窗**：删除前列出每项 + regen_hint，强化"删了能这样恢复"的安全感。
- **配色**：CSS 变量驱动的暗色 dataviz 调色板，生态色固定槽位（避免随机色误导）。

---

## 5. 安全设计（汇总）

| 维度 | 措施 | 实现位置 |
|---|---|---|
| **删除可恢复** | 一律 `trash` 移入回收站，无永久删除路径 | `delete.rs::delete_to_trash` |
| **路径白名单** | 末段必须是已知产物目录名 | `delete.rs::validate_artifact_path` |
| **防误删同名** | 目录名命中 + marker 文件双重确认 | `rules.rs::Marker` + `scan.rs::match_rule` |
| **不重复计数** | 命中目录不再深入 | `scan.rs` `skip_current_dir` |
| **跳过 .git** | 版本控制目录不进 | `scan.rs` |
| **不 follow symlink** | 防软链环与重复计数 | `walkdir(follow_links(false))` |
| **无权限容错** | 遍历错误静默跳过，不中断扫描 | `scan.rs` `Err(_) => continue` |
| **删除前二次确认** | GUI 弹窗 + CLI `confirm()` | `App.tsx` / `cli/main.rs` |
| **路径白名单（名+marker）** | 删除前 `validate_marker` 反查规则并重跑 marker | `rules.rs::validate_marker` → `delete.rs` |
| **预演不执行** | `delete_to_trash_dry_run` / `sweep clean -n` / GUI `dryRun` 入参 | `delete.rs` / `cli/main.rs` / `lib.rs` |
| **TOCTOU 防御** | 拒绝 symlink 目标 + 紧贴 trash 前复查 marker | `delete.rs::delete_to_trash_with` |
| **可取消扫描** | `AtomicBool` 贯穿 scan/compute_sizes，GUI `cancel_scan` 命令 | `scan.rs` / `lib.rs` |

---

## 6. 路线图

按"巩固差异化 → 扩生态 → 增强信任"排序。

### P0 — 巩固核心差异化（已交付）
- [x] **删除前 marker 重校验**：`validate_marker` 反查规则 + 重跑 marker（见 §4.3.1），修了原"只校验末段"的弱点。
- [x] **dry-run 模式**：`sweep clean -n` / `delete_to_trash_dry_run`（借鉴 kondo `-n`、npkill `--dry-run`）。
- [x] **trash-only 显式语义**：`delete.rs` 顶部文档注释写死安全不变量——只走 `trash::delete`，无 `remove_dir_all` 路径，不提供"永久删除"开关。

### P1 — 扩生态广度（已交付，17 类）
- [x] CMake（`build`, `cmake-build-*`，marker `CMakeLists.txt`）
- [x] .NET（`bin`, `obj`，marker `.csproj`/`.fsproj`/`.vbproj` **后缀**）
- [x] Unity（`Library`, `Temp`, `Obj`, `Logs`, `MemoryCaptures`，marker `Assembly-CSharp.csproj`）
- [x] Unreal（`Binaries`, `Intermediate`, `DerivedDataCache`, `Saved`，**suffix** `.uproject`）
- [x] Composer（`vendor`，marker `composer.json`）
- [x] Godot（`.godot`，marker `project.godot`）、Swift（`.build`/`.swiftpm`，`Package.swift`）、Zig（`zig-cache`/`.zig-cache`/`zig-out`，`build.zig`）、Elixir（`_build`/`.elixir-ls`，`mix.exs`）、CocoaPods（`Pods`，`Podfile`）
- [x] **Marker 扩 `ParentHasSuffix`**：处理文件名不固定的标记（.uproject/.csproj）
- [x] **规则表加 `risk: Risk`**（🟢 Safe / 🟡 Notice）：Artifact 带上，CLI 有"风险"列，GUI 显示徽标。借鉴 ClearDisk，强化"哪些最安全删"。

### P2 — 增强信任与精度（已交付）
- [x] **git-aware aging**：`last_active_ms` 融合 mtime + git 最后 commit 时间（取 max）。不引入 git2/libgit2（避免重依赖），直接调系统 `git log -1 --format=%ct`；git 不可用/非 git 项目自动回退纯 mtime。commit 时间比 mtime 更能反映真实开发活动。
- [x] **排除/保护路径**：`scan_artifacts` 接收 `excludes: &[String]` 前缀列表，命中前缀的产物跳过（永不清理）。CLI `scan`/`clean` 加 `--exclude`（逗号分隔、可多次）。平台无关比较（统一 `/` 分隔符）。
- [x] **扫描性能**：CLI 加 `--no-size` 跳过大小计算（快速列产物）。`--no-age` 因收益小（git log 仅在已确认产物跑、数量有限）暂不做。

### P3 — 形态与生态
- [ ] 跨平台打包：macOS（.dmg）、Linux（AppImage/deb），目前仅 NSIS（Windows）。
- [ ] `core` 作为库发布到 crates.io（对标 kondo-lib 的可集成性）。
- [ ] VS Code 扩展 / CI 集成（复用 `core`）。
- [ ] 全局缓存清理（可选模块，明确与项目产物清理分区）。

### 风险与防御
- **ClearDisk 可能移植到 Win/Linux**——其 63 路径知识库已成型，移植可行。**防御**：抢跨平台首发 + 做 Windows 原生体验（右键菜单 / Storage Sense 集成 / MFT 级快速扫描）。
- **npkill v1.0 发布多生态**——可能压缩差异化空间。**防御**：回收站默认 + marker 确认 + GUI 是 npkill 短期不会做的。

---

## 7. 项目结构

```
dev-sweeper/
├── Cargo.toml                 # workspace: core / cli / src-tauri
├── crates/
│   ├── core/                  # 扫描/删除核心（walkdir + rayon + trash），规则表驱动，单测在此
│   │   └── src/{lib,rules,scan,delete}.rs
│   └── cli/                   # sweep 命令（clap）
│       └── src/main.rs
├── src-tauri/                 # Tauri 壳：scan/delete_artifacts 命令 + 事件流
│   ├── src/{lib,main}.rs
│   ├── tauri.conf.json
│   └── capabilities/default.json
├── src/                       # React 前端（Tailwind v4）
│   ├── App.tsx
│   ├── lib/format.ts
│   └── {main.tsx,index.css}
├── README.md
└── DESIGN.md                  # 本文档
```

---

## 附录 A：技术选型理由

| 选型 | 理由 |
|---|---|
| **Rust + Tauri** | 跨平台原生 GUI、二进制小、扫描性能（rayon 并行）；规避 Electron 体积。对标 SquirrelDisk 已验证 Tauri+React+Rust 可行。 |
| **workspace 三 crate** | lib/bin 分离（学 kondo），`core` 可复用，CLI/GUI 行为一致。 |
| **walkdir + rayon** | 扫描单线程流式（快出结果），大小计算并行（吞吐）。 |
| **trash crate** | 跨平台回收站抽象，是"可恢复删除"卖点的基石。 |
| **规则表驱动** | 加生态零分支，单测易写。 |
| **React 19 + Tailwind v4** | 现代前端，CSS 变量驱动的暗色 dataviz 调色板。 |

## 附录 B：测试矩阵（`crates/core/src/lib.rs` 现有 23 测 + 1 unix-only）

| 测试 | 守住的属性 |
|---|---|
| `finds_node_modules_with_package_json` | marker 确认 + project_name 读 package.json |
| `skips_bare_target_without_marker` | 裸 target 不误报 |
| `distinguishes_rust_and_maven_target` | 同名目录按 marker 区分生态 |
| `venv_requires_pyvenv_cfg` | SelfHas marker |
| `does_not_descend_into_matched_dirs` | 嵌套产物不重复计数 |
| `skips_git_dir` | 跳过 .git |
| `dir_size_sums_files` | 大小累加正确 |
| `compute_sizes_fills_and_reports` | 流式 size 回调 |
| `delete_rejects_non_artifact_path` | 末段非产物名 → 拒绝 |
| `delete_rejects_known_name_without_marker` | 名字对但无 marker → 拒绝 |
| `validate_marker_accepts_with_marker` | node/rust/venv/cache 四类正向 |
| `dry_run_validates_but_does_not_delete` | 预演校验通过且不删 |
| `dry_run_rejects_without_marker` | 预演对无 marker 路径也拒绝 |
| `scan_respects_cancel_flag` | 取消标志生效，不扫完全部即停 |
| `scan_progress_reports_dir_count` | 进度回调报告已扫目录数 |
| `compute_sizes_skips_when_cancelled` | 取消后 size 计算跳过 |
| `does_not_follow_symlinks` (unix) | 不 follow symlink |
| `delete_rejects_symlink_target` (unix) | **TOCTOU：删除目标本身是 symlink → 拒绝** |
| `cmake_build_requires_cmakelists` | CMake 的 build 靠 CMakeLists.txt 确认（P1） |
| `dotnet_bin_obj_requires_csproj` | .NET bin/obj 靠 .csproj 后缀确认（P1，suffix marker） |
| `unreal_uses_suffix_marker` | Unreal 靠 .uproject 后缀确认（P1，suffix marker） |
| `generic_dir_without_marker_not_matched` | 通用名目录无 marker 一律不误判（P1 回归） |
| `last_active_fuses_git_commit_and_mtime` | 无 git 时回退 mtime（P2 git-aware） |
| `last_active_uses_git_commit_when_available` | 有 git 时采用 commit 时间（P2 git-aware） |
| `scan_excludes_protected_paths` | 排除前缀命中的产物被保护（P2 exclude） |
