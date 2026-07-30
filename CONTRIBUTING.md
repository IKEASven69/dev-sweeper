# 贡献指南

欢迎贡献！无论是修 bug、加生态规则、改进 UI，还是完善文档。

## 开发环境

- Rust（stable）+ Node 22 + pnpm 11
- Tauri 2 系统依赖：Linux 需 `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`，Windows/macOS 开箱即用

```bash
pnpm install
pnpm tauri dev      # 启动 GUI 开发
cargo test -p dev-sweeper-core   # 跑后端单测（改 core 后务必跑）
```

## 加一个清理生态

这是最常见的贡献。**只需改 `crates/core/src/rules.rs` 的 `RULES` 表加一行**：

```rust
CleanRule {
    id: "yourlang",
    dir_names: &["build-dir-name"],
    marker: Marker::ParentHas(&["marker-file.ext"]),  // 或 SelfHas / ParentHasSuffix / None
    regen_hint: "how to regenerate",
    risk: Risk::Safe,  // Safe=构建产物 / Notice=依赖环境
    default_on: true,
},
```

然后：前端 `src/App.tsx` 的 `RULE_META` 加对应图标/色/标签；`src/index.css` 加 `--s-yourlang` 色变量。补一个单测（见 `crates/core/src/lib.rs` 现有生态测试）。

**重要**：通用目录名（`bin`/`obj`/`build`/`vendor`…）**必须**用 marker 确认，避免误伤同名目录。

## 提交前检查

- [ ] `cargo test -p dev-sweeper-core` 通过
- [ ] `cargo clippy --workspace` 无 warning
- [ ] `pnpm exec tsc --noEmit` 通过
- [ ] 改了规则表 → 补单测

## 提交规范

Conventional Commits：`feat:` / `fix:` / `docs:` / `refactor:` / `ci:` / `test:` / `chore:`。中文 commit message 可接受。

## 安全红线

- **删除一律走回收站**（`trash::delete`），永不 `remove_dir_all`。
- **删除前必过 marker 校验**。
- 不要引入"永久删除"选项——这是工具的核心定位（见 DESIGN.md §5）。
