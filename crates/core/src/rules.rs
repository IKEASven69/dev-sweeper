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

/// 末段命中任一规则返回该规则（取表内第一条），用于删除前反查。
pub fn rule_for_dir_name(name: &str) -> Option<&'static CleanRule> {
    RULES.iter().find(|r| r.dir_names.contains(&name))
}

/// 对给定路径重跑 marker 确认：路径末段命中规则 + 该规则的 marker 仍成立。
///
/// 与扫描时 `scan::match_rule` 走相同的判定逻辑，使删除前的校验自洽——
/// 即使外部绕过 scan 直接调 `delete_to_trash`，也不会删掉"同名但无 marker"的目录。
pub fn validate_marker(path: &Path) -> Result<&'static CleanRule, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("无效路径: {}", path.display()))?;
    let rule = rule_for_dir_name(&name)
        .ok_or_else(|| format!("拒绝删除非产物目录: {}", path.display()))?;
    let ok = match rule.marker {
        Marker::ParentHas(markers) => path
            .parent()
            .is_some_and(|p| markers.iter().any(|m| p.join(m).is_file())),
        Marker::SelfHas(marker) => path.join(marker).is_file(),
        Marker::None => true,
    };
    if ok {
        Ok(rule)
    } else {
        Err(format!(
            "目录名命中但 marker 未确认，拒绝删除: {}（缺少 {}）",
            path.display(),
            marker_desc(rule.marker)
        ))
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
