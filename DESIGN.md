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

### 1.3 定位：跨生态、全舰队、可逆的空间回收器

> **一句话定位**：唯一一个跨所有语言、按陈旧度排序、只碰可重建产物且全程可逆的开发者空间回收器。

删 `node_modules` 这种单点动作是 commodity（ncdu + 一行脚本即可）。dev-sweeper 的差异化合在**五件事同时成立**：

1. **跨生态**：Node / Rust / Java / Go / Python / .NET … 一次扫描全包（规则表驱动，加生态 = 加一行）。
2. **全舰队一扫**：对整个 dev 目录/工作区批量扫描，而非一次盯一个项目——手动逐个清不现实。
3. **按陈旧度排序**：优先回收半年没碰的项目，昨天改过的碰都不碰（比 kondo 的整树 mtime 更准，并融合 git commit 时间）。
4. **只碰可重建产物**：节点_modules / target / build / .venv / __pycache__ / 各语言全局缓存——一切"能靠 lockfile / 命令重建"的东西；**永不碰源码**。
5. **全程可逆**：删除一律进回收站，依赖裁剪先备份清单，迁移先移旧产物入回收站。

其中 (3) 与 (4) 的组合（"陈旧度 × 可重建性"排序）是最锋利、竞品都没做的点；(5) 是信任基石。

### 1.4 非目标

- **不做**永久删除（`rm -rf`）—— 即使高级用户也走回收站。
- ~~**不做**全局缓存清理~~ → **已转为目标**：见 §7.6 「清全局缓存」。各语言包管理器的全局共享缓存（npm/pip/cargo/maven/gradle/go/uv/pnpm）不在项目内、普通清理碰不到，且清了只是重下——是"其他生态依赖"占盘的通用省盘杠杆，与项目内产物清理互补。
- ~~**不做**依赖分析~~ → **已转为目标（v1 Node）**：见 §7 依赖裁剪（"重构依赖"）。现可找出 `package.json` 中声明但源码从未引用的依赖并精准移除，而不只是整包删 `node_modules`。
- **不做**隐私擦除 / 系统垃圾清理（BleachBit 的领地）。
- ~~**不做压缩归档**~~ → **已转为目标**：见 §7.7 「压缩归档」。把沉睡项目整体压成 `.tar.gz`、原项目移入回收站——这是"不删也能瘦"的核心落地：活的项目的源码不删，只是压成可恢复副本移出工作区，清回收站即真正释放；正好把 §1.3 的"陈旧度排序"用起来（沉睡最久的优先回收）。

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

> **补充（2026-07 定位重定）**：在"跨平台 + 多生态 + 回收站"之上，真正的护城河是**五合一组合**——跨生态 × 全舰队一扫 × 陈旧度排序 × 只碰可重建物 × 全程可逆。竞品（DaisyDisk/ncdu 只给大小不懂生态；cargo clean/npm prune 一次一项目；CleanMyMac 不懂开发者语义）最多占其中一两格。最锋利的两点是「陈旧度 × 可重建性」排序与「数学上删不到源码」的安全人设。

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
| `analyze_deps` | `{ projectDir: String }` | `DepReport { eco, pm, declaredCount, usedCount, unused, extraneous, notes }` | — |
| `prune_deps` | `{ projectDir, remove: Vec<String>, dryRun: bool }` | `PruneReport { removed, freedBytes, backupPath, failed }` | — |
| `migrate_to_pnpm` | `{ projectDir, dryRun: bool }` | `MigrateReport { fromPm, freedBytes, backupPath, reinstalled, error }` | — |
| `migrate_to_uv` | `{ projectDir, dryRun: bool }` | `MigratePyReport { fromPm, freedBytes, backupPath, reinstalled, error }` | — |
| `discover_caches` | — | `Vec<CacheEntry> { id, eco, label, path, sizeBytes, regenHint, risk }` | — |
| `purge_cache` | `{ id: String, dryRun: bool }` | `CachePurgeReport { id, path, freedBytes, reinstallable, error }` | — |
| `discover_archivable` | `{ root: String, staleDays: u64 }` | `Vec<ArchivableProject> { name, path, sizeBytes, lastActiveMs, isGit }` | — |
| `archive_project` | `{ dir: String, archiveDir: String, dryRun: bool }` | `ArchiveReport { name, archiveFile, originalSize, compressedSize, freedBytes, removedOriginal, error }` | — |
| `list_archives` | `{ archiveDir: String }` | `Vec<ArchiveFile> { name, path, sizeBytes, projectName, createdAt }` | — |
| `restore_archive` | `{ file: String, destRoot: String, dryRun: bool }` | `RestoreReport { archiveFile, restoredTo, restoredBytes, error }` | — |

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
- `analyze_deps` / `prune_deps` / `migrate_to_pnpm`：依赖分析、裁剪、pnpm 迁移（见 §7 / §7.5），均无事件流，直接返回报告。
- `discover_caches` / `purge_cache`：全局缓存发现与清理（见 §7.6）。
- `discover_archivable` / `archive_project` / `list_archives` / `restore_archive`：沉睡项目发现、压缩归档、归档库列举、还原（见 §7.7），均无事件流，直接返回报告。

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
- [x] **依赖裁剪（"重构依赖"）**：`crates/core/src/deps.rs` 的 `analyze_deps` / `prune_deps`，CLI `sweep deps`，GUI "依赖瘦身"标签页（见 §7）。精准移除未使用依赖，而非整包删 node_modules。
- [x] **pnpm 迁移**：`deps.rs` 的 `detect_pm` / `migrate_to_pnpm`，CLI `sweep migrate`，GUI "迁移到 pnpm"入口（见 §7.5）。npm/yarn 项目迁移到 pnpm 内容寻址存储，跨项目去重省盘。
- [x] **uv 迁移（Python 版 pnpm）**：`crates/core/src/uv.rs` 的 `detect_pypm` / `migrate_to_uv`，CLI `sweep uv-migrate`，Tauri `migrate_to_uv` 命令（GUI 入口可后续接入 DepsPanel）。pip/poetry/pip-tools 项目迁移到 uv 全局内容寻址缓存 + 硬链接 venv，跨项目去重省盘——"不删也能瘦"在 Python 侧的对称杠杆（见 §7.5.1）。
- [x] **清全局缓存（通用省盘层）**：`crates/core/src/caches.rs` 的 `discover_global_caches` / `purge_cache`，CLI `sweep caches`，GUI "全局缓存"标签页（见 §7.6）。跨生态清理 npm/pip/cargo/maven/gradle/go/uv/pnpm 全局缓存，回答"其他生态依赖怎么省盘"——只动白名单内全局缓存、一律走回收站。
- [x] **压缩归档（"不删也能瘦"）**：`crates/core/src/archive.rs` 的 `discover_archivable` / `archive_project` / `list_archives` / `restore_archive`，CLI `sweep archives` / `sweep archive` / `sweep restore`，GUI "压缩归档"标签页（见 §7.7）。把沉睡项目整体压成 .tar.gz、原项目移入回收站——源码不丢、随时可还原，正好把"陈旧度排序"用起来。

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
- [x] **全局缓存清理**：已交付（见 §7.6），与项目产物清理明确分区、共用回收站安全底线。

### 风险与防御
- **ClearDisk 可能移植到 Win/Linux**——其 63 路径知识库已成型，移植可行。**防御**：抢跨平台首发 + 做 Windows 原生体验（右键菜单 / Storage Sense 集成 / MFT 级快速扫描）。
- **npkill v1.0 发布多生态**——可能压缩差异化空间。**防御**：回收站默认 + marker 确认 + GUI 是 npkill 短期不会做的。

---

## 7. 依赖裁剪（"重构依赖"）

> 动机：呼应"如果整包删了依赖，用户用什么"——dev-sweeper 不该只会删 `node_modules`，还应能**精准瘦身**：找出 `package.json` 里声明了但源码从未 `import` 的依赖，让用户移除它们而项目照跑。这与"只关心磁盘占用"的旧定位互补——清理产物管"占盘"，依赖裁剪管"清单臃肿"。

### 7.1 范围（v1）

- **Node 生态**：解析 `package.json` 的 `dependencies` / `devDependencies`，扫描源码 `import` / `require` / 动态 `import()`，算出**未使用依赖**与 **node_modules 多余目录**。
- 其他生态（Rust `cargo udeps`、Python `pip-autoremove` / `pip-check`）判定方式不同，后续按本模块结构扩展（与"加生态 = 加一行"一致）。

### 7.2 架构

- **`crates/core/src/deps.rs`**（纯分析，只读，不启动子进程，保持 core 无 IO 框架）：
  - `analyze_deps(dir) -> DepReport`：解析清单 + 扫描源码 import（正则提取说明符 → 解析为顶层包名），产出 `unused`（声明未引用）与 `extraneous`（node_modules 有但清单无）。
  - 置信度 `DepConfidence`：**`High`**＝运行期依赖从未被 import，几乎可移除；**`Review`**＝开发依赖常仅由 CLI / 配置（eslint、vite、tsc…）使用，需人工确认。
  - `prune_deps(dir, remove, dry_run) -> PruneReport`：从 `package.json` 移除指定依赖（先写 `package.json.sweep.bak` 备份），并把对应 `node_modules/<pkg>` 目录**移入回收站**（可恢复，与 `delete.rs` 同一安全底线：永不 `remove_dir_all`）。
- **CLI**：`sweep deps <path>` 列出未使用 / 多余依赖；`--apply` 确认后移除（含 `--json`）。`sweep migrate <path>` 把 npm/yarn 项目迁移到 pnpm（`-n` 预演、`-y` 跳过确认）。
- **Tauri**：`analyze_deps` / `prune_deps` / `migrate_to_pnpm` 命令；前端"依赖瘦身"标签页（`src/DepsPanel.tsx`）展示未使用依赖（带置信度徽标）、多余目录提示、当前包管理器标识，勾选后"重构依赖"或"迁移到 pnpm"（均含预演与确认弹窗）。

### 7.3 安全

- 分析只读不写；裁剪先备份清单再改，绝不直接覆盖。
- 依赖目录走回收站，误删可恢复。
- 开发依赖一律标 **需复核**，引导用户确认未被 CLI / 配置引用，降低误删运行所需依赖的风险。

### 7.4 已知局限

- 仅 Node；import 扫描基于正则，可能漏掉动态 `require`、模板字符串拼接、编译期注入的依赖；`Review` 项需人工判断。
- `extraneous` 检测为**信息性**（可能含合法的传递依赖），权威判定交给 `npm prune` / `pnpm prune`，工具仅提示。

### 7.5 pnpm 迁移（省盘杠杆）

> 动机：呼应"npm 安装的，可以通过 pnpm 安装瘦身"——dev-sweeper 扫到大量项目时，每个项目一份扁平 `node_modules` 是巨大的磁盘浪费。pnpm 用**内容寻址的全局存储**（`~/.pnpm-store`），同一份包在多个项目间只存一份，跨项目去重即省盘。这是 dev-sweeper 的"项目内产物清理"之外，跨项目维度的核心省盘杠杆。

- **包管理器识别** `detect_pm(dir) -> PmKind`：按锁文件优先级 `pnpm-lock.yaml` > `yarn.lock` > `package-lock.json` 识别（并存时以 pnpm 为准）。`DepReport.pm` 字段透出，前端据此显示"迁移到 pnpm"入口。
- **迁移动作** `migrate_to_pnpm(dir, dry_run) -> MigrateReport`：
  1. 仅当检测到 npm / yarn 锁文件才允许（Unknown 拒绝；已是 pnpm 直接拒绝）。
  2. `dry_run` 只报告"会做什么"，不移动不安装。
  3. 非空跑：先把旧 `node_modules` 与旧锁文件（`package-lock.json` / `yarn.lock`）**移入回收站**以立即释放磁盘（可恢复），再运行 `pnpm import`（基于已有 lockfile 生成 `pnpm-lock.yaml`）+ `pnpm install`。
  4. 安装命令优先全局 `pnpm`，spawn 失败（未安装）时回退 `npx --yes pnpm`，降低使用门槛。
- **安全**：与 `delete.rs` 同一底线——删除一律走回收站；`pnpm install` 失败时旧 `node_modules` 已在回收站可恢复，用户可手动 `pnpm install` 收尾，不存在"删了装不回"的死角。
- **CLI**：`sweep migrate <path>`（`-n` 预演、`-y` 跳过确认）。
- **Tauri / GUI**：`migrate_to_pnpm` 命令；前端在 `pm ∈ {npm, yarn}` 时显示"迁移到 pnpm"卡片，含预演与确认弹窗；迁移成功后刷新分析以更新 `pm` 标识。
- **测试**：`detect_pm_identifies_lockfiles`（锁文件优先级）、`migrate_rejects_unknown_pm` / `migrate_rejects_already_pnpm`（不安全拒绝）、`migrate_dry_run_reports_action`（预演）、`migrate_runs_when_pnpm_available`（条件跳过：仅当系统装了 pnpm 才真跑，校验 `pnpm-lock.yaml` 离线生成）。

#### 7.5.1 uv 迁移（Python 版 pnpm，省盘杠杆）

> 动机：与 §7.5 完全对称——`uv` 是 Python 版 pnpm：用**全局内容寻址缓存**（`~/.cache/uv`），并把每个 venv 的 `site-packages` 以**硬链接**方式指向缓存里的 wheel，从而跨项目去重。传统 `python -m venv` + `pip install` 会在每个项目里**物理拷贝**一份包（真实占盘）；迁移到 uv 后，单项目足迹≈硬链接 + 少量 `.pyc`，全局缓存只存一份——这正是 pnpm 对 Node 做的事。

- **包管理器识别** `detect_pypm(dir) -> PyPmKind`：按 `uv.lock` > `poetry.lock` > (`requirements.in` + `requirements.txt`) > (`requirements.txt` / `pyproject.toml` / `setup.py`) 识别为 `Uv` / `Poetry` / `PipTools` / `Pip` / `Unknown`。
- **迁移动作** `migrate_to_uv(dir, dry_run) -> MigratePyReport`：
  1. 仅当检测到 Python 项目（requirements.txt / pyproject.toml / setup.py）才允许（Unknown 拒绝；已是 uv 直接拒绝）。
  2. **前置守卫**：`uv_available()` 为假时直接返回 `Err`，**不移动任何文件**（uv 迁移边界更多——可编辑安装、解释器版本、锁文件语义——故比 pnpm 更保守，破坏性操作仅在确认 uv 可用后才发生）。
  3. 非空跑：先把旧 `.venv` **移入回收站**以立即释放磁盘（可恢复），再 `uv venv` 建硬链接 venv，并按清单安装：`pyproject.toml` → `uv sync`（生成 `uv.lock`）、`requirements.txt` → `uv pip install -r requirements.txt`、`setup.py` → `uv pip install -e .`。
- **安全**：与 `migrate_to_pnpm` 同一底线——删除一律走回收站；`uv sync` / `uv pip install` 失败时旧 `.venv` 已在回收站可恢复。无 uv 时零破坏（前置守卫）。
- **CLI**：`sweep uv-migrate <path>`（`-n` 预演、`-y` 跳过确认）。
- **Tauri / GUI**：`migrate_to_uv` 命令已提供（GUI 入口可后续接入 DepsPanel）。
- **测试**：`detect_pypm_basics`（识别四种 PM）、`migrate_rejects_non_python` / `migrate_rejects_already_uv`（不安全拒绝）、`migrate_dry_run_reports_action`（预演）、`migrate_guards_without_uv`（条件跳过：无 uv 时返回 Err 且不创建 `.venv`，零破坏）。

### 7.6 清全局缓存（通用省盘杠杆）

> 动机：呼应"pnpm 只能去重 node，那其他生态依赖怎么办"——Node 是唯一默认不跨项目去重的（每个项目一份扁平 node_modules）；而 **JVM（~/.m2 / ~/.gradle）、Go（GOPATH/pkg/mod）、Rust（~/.cargo，源码已全局共享）的依赖默认就全局共享**；Python 的 `uv` 是 Python 版 pnpm。因此"其他生态依赖占盘"的通用答案不是迁移，而是**清各语言包管理器的全局缓存**——它们都在项目之外、清了只是重下、不丢源码。这是与"项目内产物清理"互补的、跨生态通用的省盘层。

- **白名单** `crates/core/src/caches.rs`（`discover_global_caches`）：维护跨生态全局缓存清单（逐平台多路径提示，取第一个存在的），覆盖 `node-npm-cache` / `rust-cargo-registry` / `rust-cargo-git` / `java-maven` / `java-gradle` / `go-mod` / `python-pip` / `python-uv` / `pnpm-store`。
- **路径解析**：`~` 与 `${KEY}` 占位符按 `home` + 环境变量表解析；环境变量缺失时回退到各包管理器的"默认位置"（如 `CARGO_HOME`→`~/.cargo`、`GOPATH`→`~/go`、`M2_REPO`→`~/.m2/repository`）。
- **清理动作** `purge_cache(id, dry_run) -> CachePurgeReport`：先算 `freed_bytes`（置于 trash 之前），再 `trash::delete` 移入回收站（可恢复，与 `delete.rs` 同一安全底线）；`dry_run` 只报告。
- **风险分级**：纯下载缓存（npm/pip/cargo/gradle/maven/go）标 `safe`；会损失跨项目去重或需重装的（uv / pnpm store）标 `notice`——前端据此显示 🟢/🟡 徽标，引导用户"清 pnpm store 会暂时失去去重收益"。
- **安全**：发现阶段只列存在的缓存、不删；清理只动白名单内的全局缓存目录（非任意路径），一律走回收站。
- **CLI**：`sweep caches` 列出全部（含大小/风险/路径，`--json` 供脚本）；`--id <id>` 清理指定（可重复）、`--apply` 清理全部；`-n` 预演。非交互终端（无 TTY）不弹确认，需显式 `-n` / `--json`。
- **Tauri / GUI**：`discover_caches` / `purge_cache` 命令；前端"全局缓存"标签页（`src/CachesPanel.tsx`）自动发现、勾选后"清理全部/清理选中"（含预演与确认弹窗）。
- **测试**：`resolve_expands_tilde_and_vars`（路径解析）、`discover_returns_unique_ids`（发现不重复）、`purge_path_dry_run_touches_nothing`（预演不动）、`purge_path_real_does_not_panic`（真实跑不 panic，沙箱无回收站时优雅报错）。

### 7.7 压缩归档（"不删也能瘦"）

> 动机：呼应"活的又不能删，只能压缩"——dev-sweeper 不该只会删产物或只管依赖，还应能**把整个沉睡项目压成可恢复的归档**，让源码留着（可还原）却不再占工作区的即时空间。配合"按陈旧度排序"，把"睡得最久的项目优先回收"这个差异化卖点真正落地。

- **发现沉睡项目** `discover_archivable(root, min_stale_days) -> Vec<ArchivableProject>`：扫描 root 下一级子目录（跳隐藏目录、符号链接、非目录），按"最后活跃时间"升序（最旧在前）；`min_stale_days` 过滤只返回超过 N 天未活跃的项目（0 = 全部）。
- **归档动作** `archive_project(dir, archive_dir, dry_run) -> ArchiveReport`：
  1. `dry_run` 只报告"会打包 + 释放多少"，不写文件不删项目。
  2. 非空跑：把整个项目打成 `<name>@<日期>.tar.gz`（tar + gzip），写入 `archive_dir`（默认 `~/dev-archives`）；再 `trash::delete` 把**原项目移入回收站**（可恢复，与 `delete.rs` 同一安全底线）。归档文件本身也是一份可恢复副本。
  3. 要真正释放磁盘，用户清空回收站即可（与"删 node_modules"行为统一）——归档不含源码，还原随时可用。
- **归档库管理** `list_archives(archive_dir)` 列举已有 `.tar.gz`；**还原** `restore_archive(file, dest_root, dry_run)`：把归档解回 `dest_root/<project_name>`（已存在则报错避免覆盖），`dry_run` 只校验。
- **安全**：归档只读写指定目录，绝不强删；原项目走回收站；归档文件名带日期便于追溯。
- **CLI**：
  - `sweep archives --root <dir> [--stale-days N]` 发现沉睡项目（列表 / `--json`）；
  - `sweep archive <path> [--archive-dir DIR] [-n] [-y]` 归档单个项目；
  - `sweep restore <file> [--dest DIR] [-n]` 解回。
- **Tauri / GUI**：`discover_archivable` / `archive_project` / `list_archives` / `restore_archive` 命令；前端"压缩归档"标签页（`src/ArchivePanel.tsx`）展示沉睡项目（按陈旧度）、勾选后归档，并管理归档库（列出 + 一键还原），均含预演与确认弹窗。
- **测试**：`discover_skips_hidden_symlinks_and_files`（跳隐藏/符号链接/文件）、`discover_excludes_fresh_when_stale_filter`（陈旧阈值过滤）、`archive_and_restore_roundtrip`（打包→列出→还原、文件完整）、`restore_dry_run_does_nothing`（预演不动）、`parse_names`（文件名解析）。

## 8. 项目结构

```
dev-sweeper/
├── Cargo.toml                 # workspace: core / cli / src-tauri
├── crates/
│   ├── core/                  # 扫描/删除核心（walkdir + rayon + trash），规则表驱动，单测在此
│   │   └── src/{lib,rules,scan,delete,deps,caches,archive}.rs
│   └── cli/                   # sweep 命令（clap）
│       └── src/main.rs
├── src-tauri/                 # Tauri 壳：scan/delete_artifacts/analyze_deps/prune_deps/migrate_to_pnpm/migrate_to_uv/discover_caches/purge_cache/discover_archivable/archive_project/list_archives/restore_archive 命令 + 事件流
│   ├── src/{lib,main}.rs
│   ├── tauri.conf.json
│   └── capabilities/default.json
├── src/                       # React 前端（Tailwind v4）
│   ├── App.tsx                # 顶栏 + 模式切换（清理产物 / 依赖瘦身 / 全局缓存 / 压缩归档）
│   ├── DepsPanel.tsx          # "依赖瘦身"标签页
│   ├── CachesPanel.tsx        # "全局缓存"标签页
│   ├── ArchivePanel.tsx       # "压缩归档"标签页
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

## 附录 B：测试矩阵（core crate 现有 44 测，lib.rs 23 + deps.rs 12 + caches.rs 4 + archive.rs 5；其中 1 unix-only）

`lib.rs` 扫描/删除相关（见上表）。`deps.rs` 依赖裁剪与 pnpm 迁移相关：

| 测试 | 守住的属性 |
|---|---|
| `resolve_package_name_basics` | 相对/绝对/内置/URL → None；scope 包取前两段 |
| `extract_import_specifiers_variants` | import/require/动态 import 各种写法均提取 |
| `analyze_finds_unused_runtime_dep` | 运行期未使用→High、开发期未使用→Review |
| `analyze_reports_extraneous` | node_modules 顶层多余目录被识别 |
| `prune_removes_from_manifest_and_trashes_dir` | 裁剪改写清单 + 写 .bak + 移回收站 |
| `prune_dry_run_touches_nothing` | 预演不写文件不移动 |
| `analyze_rejects_non_node` | 非 Node 项目返回 Err |
| `detect_pm_identifies_lockfiles` | pnpm-lock > yarn.lock > package-lock 优先级 |
| `migrate_rejects_unknown_pm` | 无锁文件拒绝迁移 |
| `migrate_rejects_already_pnpm` | 已是 pnpm 拒绝迁移 |
| `migrate_dry_run_reports_action` | 预演只报告动作 |
| `migrate_runs_when_pnpm_available` | 条件跳过：装了 pnpm 才跑，校验 pnpm-lock.yaml 生成 |
| `detect_pypm_basics` | 识别 pip/uv/poetry/pip-tools |
| `migrate_rejects_non_python` | 非 Python 项目拒绝迁移 |
| `migrate_rejects_already_uv` | 已是 uv 拒绝迁移 |
| `migrate_guards_without_uv` | 无 uv 前置守卫拒绝且不创建 .venv |
| `resolve_expands_tilde_and_vars` | `~` 与 `${KEY}` 占位符解析正确 |
| `discover_returns_unique_ids` | 发现的全局缓存 id 不重复 |
| `purge_path_dry_run_touches_nothing` | 预演算大小但不移动 |
| `purge_path_real_does_not_panic` | 真实清理不 panic，沙箱无回收站时优雅报错 |

| 测试 | 守住的属性 |
|---|---|
| `discover_skips_hidden_symlinks_and_files` | 跳隐藏/符号链接/文件，只留下正常项目目录 |
| `discover_excludes_fresh_when_stale_filter` | 陈旧阈值过滤掉刚创建的项目 |
| `archive_and_restore_roundtrip` | 打包→列出→还原完整、压缩产出有效 |
| `restore_dry_run_does_nothing` | 预演只校验不写 |
| `parse_names` | 归档文件名解析出项目名/日期 |

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
