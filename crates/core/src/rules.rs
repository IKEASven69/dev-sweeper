//! 清理规则表：加生态 = 加一行规则。

use std::path::Path;

/// 标记确认方式，防止误删同名目录（如无 Cargo.toml/pom.xml 的裸 target）。
#[derive(Clone, Copy, Debug)]
pub enum Marker {
    /// 父目录含任一标记文件才命中
    ParentHas(&'static [&'static str]),
    /// 目录自身含标记文件才命中（如 venv 的 pyvenv.cfg）
    SelfHas(&'static str),
    /// 无需确认
    None,
}

#[derive(Clone, Copy, Debug)]
pub struct CleanRule {
    pub id: &'static str,
    pub dir_names: &'static [&'static str],
    pub marker: Marker,
    pub regen_hint: &'static str,
    pub default_on: bool,
}

/// 同名目录（如 target）按表内顺序取第一条确认成功的规则。
pub const RULES: &[CleanRule] = &[
    CleanRule {
        id: "node",
        dir_names: &["node_modules"],
        marker: Marker::ParentHas(&["package.json"]),
        regen_hint: "npm install / pnpm install",
        default_on: true,
    },
    CleanRule {
        id: "rust",
        dir_names: &["target"],
        marker: Marker::ParentHas(&["Cargo.toml"]),
        regen_hint: "cargo build",
        default_on: true,
    },
    CleanRule {
        id: "maven",
        dir_names: &["target"],
        marker: Marker::ParentHas(&["pom.xml"]),
        regen_hint: "mvn package",
        default_on: true,
    },
    CleanRule {
        id: "gradle",
        dir_names: &["build", ".gradle"],
        marker: Marker::ParentHas(&[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ]),
        regen_hint: "gradle build",
        default_on: true,
    },
    CleanRule {
        id: "python-venv",
        dir_names: &[".venv", "venv", "env"],
        marker: Marker::SelfHas("pyvenv.cfg"),
        regen_hint: "python -m venv / uv sync",
        default_on: true,
    },
    CleanRule {
        id: "python-cache",
        dir_names: &["__pycache__", ".pytest_cache"],
        marker: Marker::None,
        regen_hint: "运行时自动再生",
        default_on: true,
    },
    CleanRule {
        id: "web-dist",
        dir_names: &[".next", "dist"],
        marker: Marker::ParentHas(&["package.json"]),
        regen_hint: "npm run build",
        default_on: false,
    },
];

/// 按 id 选规则；传空列表则返回所有 default_on 的规则。未知 id 忽略。
pub fn select_rules(ids: &[String]) -> Vec<&'static CleanRule> {
    if ids.is_empty() {
        RULES.iter().filter(|r| r.default_on).collect()
    } else {
        RULES.iter().filter(|r| ids.iter().any(|i| i == r.id)).collect()
    }
}

/// 所有规则覆盖的目录名，用于删除前校验。
pub fn known_dir_names() -> Vec<&'static str> {
    RULES.iter().flat_map(|r| r.dir_names.iter().copied()).collect()
}

/// 末段命中任一规则返回该规则（取表内第一条），仅用于"这个名字是否属于某规则"的快速判断。
pub fn rule_for_dir_name(name: &str) -> Option<&'static CleanRule> {
    RULES.iter().find(|r| r.dir_names.contains(&name))
}

/// 对给定路径重跑 marker 确认：在所有"目录名命中"的规则中，取第一条 marker 也通过的。
///
/// 与扫描时 `scan::match_rule` 走**相同**的判定逻辑——同名目录（如 `target` 同时被
/// rust/maven 声明）按表内顺序逐条检查 marker，确保"扫描能识别 → 删除能通过"。
/// 这样即使外部绕过 scan 直接调 `delete_to_trash`，校验也与扫描一致。
pub fn validate_marker(path: &Path) -> Result<&'static CleanRule, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("无效路径: {}", path.display()))?
        .to_string_lossy();
    // 收集所有目录名命中的规则（target 会命中 rust 和 maven 两条）
    let hits: Vec<&'static CleanRule> = RULES
        .iter()
        .filter(|r| r.dir_names.contains(&name.as_ref()))
        .collect();
    if hits.is_empty() {
        return Err(format!("拒绝删除非产物目录: {}", path.display()));
    }
    // 取第一条 marker 也通过的——与 scan::match_rule 一致
    for rule in &hits {
        if marker_ok(rule, path) {
            return Ok(rule);
        }
    }
    // 全部命中规则但无一条 marker 通过：报错时列出所有候选项的期望 marker
    let expected = hits
        .iter()
        .map(|r| marker_desc(r.marker))
        .collect::<Vec<_>>()
        .join(" 或 ");
    Err(format!(
        "目录名命中但 marker 未确认，拒绝删除: {}（期望 {}）",
        path.display(),
        expected
    ))
}

/// 判定某规则的 marker 在给定路径上是否成立。
///
/// 扫描（`scan::match_rule`）与删除前校验（`validate_marker`）共用此函数，
/// 确保"能扫出来 ⟺ 能通过删除校验"，杜绝扫描/删除语义分裂。
pub(crate) fn marker_ok(rule: &CleanRule, path: &Path) -> bool {
    match rule.marker {
        Marker::ParentHas(markers) => path
            .parent()
            .is_some_and(|p| markers.iter().any(|m| p.join(m).is_file())),
        Marker::SelfHas(marker) => path.join(marker).is_file(),
        Marker::None => true,
    }
}

/// 生成可读的 marker 描述，用于错误信息。
fn marker_desc(marker: Marker) -> String {
    match marker {
        Marker::ParentHas(ms) => {
            let list = ms.join(" / ");
            format!("父目录标记 {list}")
        }
        Marker::SelfHas(m) => format!("目录内 {m}"),
        Marker::None => "（无标记）".into(),
    }
}
