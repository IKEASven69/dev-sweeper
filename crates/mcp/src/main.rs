//! sweep-mcp：dev-sweeper 的 MCP server（stdio 传输）。
//!
//! 协议层用官方 Rust SDK rmcp；工具逻辑在 [`ops`]（纯函数，可单测）。
//! 所有破坏性操作（clean / caches_purge）受双开关守卫：真实执行必须
//! dry_run=false 且 confirm=true，缺一即拒绝并把原因返回给调用方。

mod ops;

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_handler, tool_router,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    transport::io::stdio,
};
use serde_json::Value;

use crate::ops::{ArchivesArgs, CleanArgs, DepsArgs, PurgeArgs, ScanArgs};

#[derive(Clone, Default)]
pub struct SweepServer;

/// 把 ops 的 Ok(Value) 包装成成功结果（pretty JSON 文本，agent 可直接解析）。
fn ok_json(v: Value) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
    )]))
}

/// 把 ops 的 Err(String) 包装成工具级错误（isError=true，消息直达 agent）。
/// 这是"工具跑了但被守卫拒绝/执行失败"，不是协议错误。
fn err_text(msg: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
}

/// 在阻塞线程池里跑 FS 密集型 core 调用，避免长时间扫描卡住 async runtime。
/// Ok(Value) → 成功结果；Err(String) → 工具级错误（agent 可见）。
async fn call_tool_json<F>(f: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => ok_json(v),
        Ok(Err(msg)) => err_text(msg),
        Err(e) => Err(McpError::internal_error(format!("任务执行失败: {e}"), None)),
    }
}

#[tool_router]
impl SweepServer {
    #[tool(description = "扫描开发产物（node_modules / target / venv / __pycache__ 等），返回产物清单 JSON（路径、体积、最后活跃、风险等级）。只读，不做任何修改。")]
    async fn scan(
        &self,
        Parameters(args): Parameters<ScanArgs>,
    ) -> Result<CallToolResult, McpError> {
        call_tool_json(move || {
            let artifacts = ops::scan_and_size(&args)?;
            serde_json::to_value(artifacts).map_err(|e| e.to_string())
        })
        .await
    }

    #[tool(description = "把扫描到的产物移入回收站（可恢复）。默认 dry_run=true 只预演：返回会删什么/释放多少/哪些被校验拒绝。真实执行必须同时传 dry_run=false 且 confirm=true，缺一即拒绝。")]
    async fn clean(
        &self,
        Parameters(args): Parameters<CleanArgs>,
    ) -> Result<CallToolResult, McpError> {
        call_tool_json(move || ops::clean(&args)).await
    }

    #[tool(description = "分析项目依赖（须含 package.json），返回未使用/多余依赖报告 JSON。只读分析，不修改。")]
    async fn deps(
        &self,
        Parameters(args): Parameters<DepsArgs>,
    ) -> Result<CallToolResult, McpError> {
        call_tool_json(move || ops::deps(&args)).await
    }

    #[tool(description = "发现各语言全局依赖缓存（npm/pip/cargo/maven/gradle/go/uv/pnpm…），返回 id、体积、风险与再生成提示。只读。")]
    async fn caches(&self) -> Result<CallToolResult, McpError> {
        call_tool_json(|| Ok(ops::caches())).await
    }

    #[tool(description = "按 id 清理全局缓存（移入回收站）。默认 dry_run=true 只预演；真实执行必须同时传 dry_run=false 且 confirm=true，缺一即拒绝。")]
    async fn caches_purge(
        &self,
        Parameters(args): Parameters<PurgeArgs>,
    ) -> Result<CallToolResult, McpError> {
        call_tool_json(move || ops::caches_purge(&args)).await
    }

    #[tool(description = "发现沉睡项目（根目录下一级子目录，按最后活跃升序），用于评估归档候选。只读。")]
    async fn archives_discover(
        &self,
        Parameters(args): Parameters<ArchivesArgs>,
    ) -> Result<CallToolResult, McpError> {
        call_tool_json(move || ops::archives_discover(&args)).await
    }
}

#[tool_handler]
impl ServerHandler for SweepServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("sweep-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "dev-sweeper MCP：扫描/清理开发产物（node_modules、target、venv 等）。\n\
                 安全模型：删除一律移入回收站（可恢复）；产物识别需规则名+项目标记双重确认；\n\
                 clean 与 caches_purge 真实执行必须 dry_run=false 且 confirm=true 双开关，\n\
                 缺一即拒绝。建议流程：scan/caches 查看清单 → 预演 → 确认后真实执行。",
            )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = SweepServer.serve(stdio()).await.inspect_err(|e| {
        eprintln!("sweep-mcp: 启动失败: {e}");
    })?;
    service.waiting().await?;
    Ok(())
}
