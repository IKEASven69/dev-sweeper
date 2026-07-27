//! 清理规则表：加生态 = 加一行规则。

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
