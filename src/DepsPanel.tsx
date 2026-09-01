import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { fmtSize } from "./lib/format";

interface DepEntry {
  name: string;
  version: string | null;
  kind: "runtime" | "dev";
  status: "used" | "unused" | "extraneous";
  confidence: "high" | "review";
  note: string | null;
}

interface DepReport {
  eco: "node" | "unknown";
  pm: "npm" | "yarn" | "pnpm" | "unknown";
  projectDir: string;
  projectName: string;
  declaredCount: number;
  usedCount: number;
  unused: DepEntry[];
  extraneous: DepEntry[];
  notes: string[];
}

interface PruneReport {
  removed: string[];
  freedBytes: number;
  backupPath: string | null;
  failed: [string, string][];
  dryRun: boolean;
}

interface MigrateReport {
  fromPm: "npm" | "yarn" | "pnpm" | "unknown";
  freedBytes: number;
  backupPath: string | null;
  reinstalled: boolean;
  error: string | null;
  dryRun: boolean;
}

/** "依赖瘦身"面板：分析单个 Node 项目，列出未使用/多余依赖，可勾选后精准移除。 */
export default function DepsPanel({ projectDir }: { projectDir: string }) {
  const [report, setReport] = useState<DepReport | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<PruneReport | null>(null);
  const [confirming, setConfirming] = useState(false);

  // pnpm 迁移相关状态
  const [migrating, setMigrating] = useState(false);
  const [migrateResult, setMigrateResult] = useState<MigrateReport | null>(null);
  const [migrateConfirming, setMigrateConfirming] = useState(false);

  // 项目目录变化时清空旧结果
  useEffect(() => {
    setReport(null);
    setResult(null);
    setError(null);
    setSelected(new Set());
    setMigrateResult(null);
    setMigrateConfirming(false);
    setMigrating(false);
  }, [projectDir]);

  // 响应顶栏"分析依赖"按钮
  useEffect(() => {
    function onAnalyze() {
      void analyze();
    }
    window.addEventListener("dev-sweeper:analyze", onAnalyze);
    return () => window.removeEventListener("dev-sweeper:analyze", onAnalyze);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectDir]);

  useEffect(() => {
    if (!result) return;
    const t = setTimeout(() => setResult(null), 8000);
    return () => clearTimeout(t);
  }, [result]);

  useEffect(() => {
    if (!migrateResult) return;
    const t = setTimeout(() => setMigrateResult(null), 8000);
    return () => clearTimeout(t);
  }, [migrateResult]);

  async function analyze() {
    if (!projectDir) return;
    setAnalyzing(true);
    setError(null);
    setResult(null);
    setSelected(new Set());
    try {
      const r = await invoke<DepReport>("analyze_deps", { projectDir });
      setReport(r);
    } catch (e) {
      setError(String(e));
      setReport(null);
    } finally {
      setAnalyzing(false);
    }
  }

  const selectedItems = useMemo(
    () => (report ? report.unused.filter((d) => selected.has(d.name)) : []),
    [report, selected],
  );

  async function doPrune(dry: boolean) {
    if (!report) return;
    const names = selectedItems.map((d) => d.name);
    if (names.length === 0) return;
    setBusy(true);
    try {
      const r = await invoke<PruneReport>("prune_deps", {
        projectDir,
        remove: names,
        dryRun: dry,
      });
      setResult(r);
      setReport(null); // 清单已变，需重新分析
      setSelected(new Set());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  }

  async function doMigrate(dry: boolean) {
    if (!report) return;
    setMigrating(true);
    try {
      const r = await invoke<MigrateReport>("migrate_to_pnpm", {
        projectDir,
        dryRun: dry,
      });
      setMigrateResult(r);
      if (!r.dryRun && r.reinstalled) {
        setReport(null); // 已迁移，需重新分析以刷新 pm 标识
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setMigrating(false);
      setMigrateConfirming(false);
    }
  }

  function toggle(name: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  function toggleAll() {
    if (!report) return;
    setSelected(
      selected.size === report.unused.length
        ? new Set()
        : new Set(report.unused.map((d) => d.name)),
    );
  }

  function pickDir() {
    open({ directory: true, defaultPath: projectDir || undefined }).then((dir) => {
      if (typeof dir === "string") {
        // 通过自定义事件通知 App 更新 root（与清理模式共用同一目录状态）
        window.dispatchEvent(new CustomEvent("dev-sweeper:set-root", { detail: dir }));
      }
    });
  }

  const allSelected = report != null && report.unused.length > 0 && selected.size === report.unused.length;

  return (
    <div className="flex-1 overflow-auto px-6 pb-6">
      <div className="rounded-xl border border-[var(--hairline)] bg-[var(--surface)] overflow-hidden min-h-[280px]">
        {/* 工具条 */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-[var(--grid)]">
          <button
            onClick={pickDir}
            className="px-3 py-1.5 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
          >
            选择项目目录
          </button>
          <span className="text-xs text-[var(--muted)] truncate" title={projectDir}>
            {projectDir || "未选择目录"}
          </span>
          <div className="flex-1" />
          <button
            onClick={analyze}
            disabled={!projectDir || analyzing}
            className="px-4 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium"
          >
            {analyzing ? "分析中…" : "分析依赖"}
          </button>
        </div>

        {error && (
          <div className="px-4 py-3 text-sm" style={{ color: "var(--critical)" }}>
            ✕ {error}
          </div>
        )}

        {!report && !error && !analyzing && (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-[var(--muted)] py-24">
            <span className="text-3xl">🧩</span>
            <span className="text-sm">选择含 package.json 的项目目录，点击「分析依赖」</span>
            <span className="text-xs">找出声明了但源码从未用到的依赖，精准瘦身而非整包删除</span>
          </div>
        )}

        {report && (
          <div className="p-4 space-y-4">
            <div className="flex items-center gap-3 text-sm">
              <div className="flex-1 text-[var(--ink-2)]">
                项目 <span className="font-medium text-[var(--ink-1)]">{report.projectName}</span> ·
                声明 <b>{report.declaredCount}</b> 个依赖，
                其中 <b>{report.usedCount}</b> 个被源码引用
              </div>
              {report.unused.length > 0 && (
                <button
                  onClick={toggleAll}
                  className="px-2.5 py-1 rounded-md border border-[var(--hairline)] hover:border-[var(--baseline)] text-xs text-[var(--ink-2)]"
                >
                  {allSelected ? "取消全选" : "全选未使用项"}
                </button>
              )}
            </div>

            {/* pnpm 迁移建议 */}
            {report.pm === "npm" || report.pm === "yarn" ? (
              <div className="rounded-lg border border-[var(--hairline)] px-3 py-2.5 flex items-center gap-3">
                <div className="flex-1 min-w-0">
                  <div className="text-sm text-[var(--ink-2)]">
                    当前包管理器：
                    <span className="font-medium text-[var(--ink-1)]">
                      {report.pm === "npm" ? "npm" : "yarn"}
                    </span>
                  </div>
                  <div className="text-xs text-[var(--muted)] mt-0.5">
                    迁移到 pnpm 可用内容寻址全局存储跨项目去重，显著省磁盘（dev-sweeper 扫大量项目时的核心省盘杠杆）。
                  </div>
                </div>
                <button
                  onClick={() => setMigrateConfirming(true)}
                  disabled={migrating}
                  className="px-3 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-40 text-sm font-medium shrink-0"
                >
                  迁移到 pnpm
                </button>
              </div>
            ) : report.pm === "pnpm" ? (
              <div className="rounded-lg border border-[var(--hairline)] px-3 py-2.5 text-sm text-[var(--ink-2)]">
                当前已是 <span className="font-medium text-[var(--ink-1)]">pnpm</span>，享受内容寻址存储的跨项目去重省盘。
              </div>
            ) : null}

            {/* 未使用依赖 */}
            {report.unused.length === 0 ? (
              <div className="text-sm text-[var(--muted)] py-6 text-center">
                ✓ 未发现未使用的声明依赖。
              </div>
            ) : (
              <div className="rounded-lg border border-[var(--grid)] divide-y divide-[var(--grid)]">
                {report.unused.map((d) => {
                  const sel = selected.has(d.name);
                  return (
                    <div
                      key={d.name}
                      className={`flex items-center gap-3 px-3 py-2.5 cursor-pointer ${
                        sel ? "bg-white/[0.04]" : "hover:bg-white/[0.03]"
                      }`}
                      onClick={() => toggle(d.name)}
                    >
                      <input
                        type="checkbox"
                        checked={sel}
                        onChange={() => toggle(d.name)}
                        onClick={(e) => e.stopPropagation()}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="font-medium text-[var(--ink-1)] truncate">{d.name}</span>
                          {d.version && (
                            <span className="text-xs text-[var(--muted)]">{d.version}</span>
                          )}
                          <span
                            className="text-[10px] px-1.5 py-0.5 rounded shrink-0"
                            style={{ background: "rgba(255,255,255,0.05)", color: "var(--muted)" }}
                          >
                            {d.kind === "runtime" ? "运行" : "开发"}
                          </span>
                        </div>
                        {d.note && (
                          <div className="text-xs text-[var(--muted)] mt-0.5 truncate">{d.note}</div>
                        )}
                      </div>
                      {d.confidence === "high" ? (
                        <span
                          className="text-[10px] px-1.5 py-0.5 rounded shrink-0"
                          style={{ color: "var(--good)", background: "rgba(60,200,120,0.12)" }}
                        >
                          高
                        </span>
                      ) : (
                        <span
                          className="text-[10px] px-1.5 py-0.5 rounded shrink-0"
                          style={{ color: "var(--warning)", background: "rgba(250,178,25,0.12)" }}
                        >
                          需复核
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            )}

            {/* 多余依赖（提示性） */}
            {report.extraneous.length > 0 && (
              <div className="rounded-lg border border-[var(--hairline)] px-3 py-2.5">
                <div className="text-xs text-[var(--muted)] mb-1">
                  node_modules 中不在 package.json 的目录（{report.extraneous.length} 个，可能含传递依赖）：
                </div>
                <div className="text-xs text-[var(--ink-2)] break-words">
                  {report.extraneous.map((d) => d.name).join("、")}
                </div>
                <div className="text-xs text-[var(--muted)] mt-1">
                  建议运行 <code>npm prune</code> / <code>pnpm prune</code> 让包管理器权威判定。
                </div>
              </div>
            )}

            {/* 备注 */}
            {report.notes.map((n, i) => (
              <div key={i} className="text-xs text-[var(--muted)]">
                · {n}
              </div>
            ))}

            {/* 操作 */}
            {report.unused.length > 0 && (
              <div className="flex items-center gap-3 pt-1">
                <div className="flex-1" />
                <button
                  onClick={() => setConfirming(true)}
                  disabled={selected.size === 0 || busy}
                  className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium"
                >
                  重构依赖（{selected.size}）
                </button>
              </div>
            )}
          </div>
        )}
      </div>

      {/* 结果 toast */}
      {result && (
        <div className="toast-in fixed bottom-5 right-5 z-20 rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3 text-sm shadow-xl max-w-md">
          {result.dryRun ? (
            <>
              <span style={{ color: "var(--accent)" }}>🔍</span> 预演：本会移除 {result.removed.length}{" "}
              个依赖，实际未改动
              {result.failed.length > 0 && (
                <div className="mt-1 text-xs" style={{ color: "var(--critical)" }}>
                  ✕ {result.failed.length} 项失败：{result.failed[0][1]}
                </div>
              )}
            </>
          ) : (
            <>
              <span style={{ color: "var(--good)" }}>✓</span> 已移除 {result.removed.length} 个依赖，
              释放 {fmtSize(result.freedBytes)}
              {result.backupPath && (
                <div className="text-xs text-[var(--muted)] mt-0.5">
                  备份：{result.backupPath}
                </div>
              )}
              {result.failed.length > 0 && (
                <div className="mt-1 text-xs" style={{ color: "var(--critical)" }}>
                  ✕ {result.failed.length} 项失败：{result.failed[0][1]}
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* pnpm 迁移结果 toast */}
      {migrateResult && (
        <div className="toast-in fixed bottom-5 right-5 z-20 rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3 text-sm shadow-xl max-w-md">
          {migrateResult.dryRun ? (
            <>
              <span style={{ color: "var(--accent)" }}>🔍</span> 预演：会把{" "}
              {migrateResult.fromPm === "npm" ? "npm" : "yarn"} 项目迁移到 pnpm，旧 node_modules 移入回收站后重建，实际未改动
            </>
          ) : migrateResult.reinstalled ? (
            <>
              <span style={{ color: "var(--good)" }}>✓</span> 已迁移到 pnpm，释放{" "}
              {fmtSize(migrateResult.freedBytes)}，旧 node_modules 已移入回收站
              {migrateResult.backupPath && (
                <div className="text-xs text-[var(--muted)] mt-0.5">
                  回收站原路径：{migrateResult.backupPath}
                </div>
              )}
            </>
          ) : (
            <>
              <span style={{ color: "var(--critical)" }}>✕</span> 迁移未完成：
              {migrateResult.error ?? "未知错误"}
              <div className="text-xs text-[var(--muted)] mt-0.5">
                旧 node_modules 已移入回收站，可恢复后手动 `pnpm install`
              </div>
            </>
          )}
        </div>
      )}

      {/* pnpm 迁移确认对话框 */}
      {migrateConfirming && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center p-8 z-10">
          <div className="bg-[var(--surface)] border border-[var(--hairline)] rounded-2xl max-w-xl w-full max-h-[80vh] flex flex-col shadow-2xl">
            <div className="px-5 py-4 border-b border-[var(--grid)]">
              <h2 className="font-semibold">迁移到 pnpm（从 {report?.pm === "npm" ? "npm" : "yarn"}）</h2>
              <p className="text-sm text-[var(--muted)] mt-1">
                旧 node_modules 与旧锁文件会先移入回收站（可恢复），立即释放磁盘；随后运行{" "}
                <code>pnpm import</code> + <code>pnpm install</code> 重建依赖。pnpm 的内容寻址全局存储可跨项目去重，省出大量空间。
              </p>
            </div>
            <div className="px-5 py-4 border-t border-[var(--grid)] flex items-center gap-3 justify-end">
              {migrating && <div className="mr-auto text-xs text-[var(--muted)]">处理中…</div>}
              <button
                onClick={() => setMigrateConfirming(false)}
                disabled={migrating}
                className="px-4 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
              >
                取消
              </button>
              <button
                onClick={() => doMigrate(true)}
                disabled={migrating}
                className="px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--accent)] hover:text-[var(--accent)] text-sm text-[var(--ink-2)]"
              >
                {migrating ? "校验中…" : "先预演"}
              </button>
              <button
                onClick={() => doMigrate(false)}
                disabled={migrating}
                className="px-4 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-50 text-sm font-medium"
              >
                {migrating ? "迁移中…" : "确认迁移"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 确认对话框 */}
      {confirming && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center p-8 z-10">
          <div className="bg-[var(--surface)] border border-[var(--hairline)] rounded-2xl max-w-2xl w-full max-h-[80vh] flex flex-col shadow-2xl">
            <div className="px-5 py-4 border-b border-[var(--grid)]">
              <h2 className="font-semibold">
                重构依赖：移除 {selectedItems.length} 个未使用依赖
              </h2>
              <p className="text-sm text-[var(--muted)] mt-1">
                package.json 会先备份为 package.json.sweep.bak；对应 node_modules 目录移入回收站（可恢复）。
                运行期依赖（高）可较放心移除，开发依赖（需复核）请先确认未被 CLI/配置引用。
              </p>
            </div>
            <div className="flex-1 overflow-auto px-5 py-3 text-sm space-y-2">
              {selectedItems.map((d) => (
                <div key={d.name} className="flex items-center gap-3">
                  <span className="font-medium text-[var(--ink-1)]">{d.name}</span>
                  <span
                    className="text-[10px] px-1.5 py-0.5 rounded"
                    style={{ background: "rgba(255,255,255,0.05)", color: "var(--muted)" }}
                  >
                    {d.kind === "runtime" ? "运行" : "开发"}
                  </span>
                  {d.confidence === "high" ? (
                    <span className="text-[10px] px-1.5 py-0.5 rounded" style={{ color: "var(--good)", background: "rgba(60,200,120,0.12)" }}>
                      高
                    </span>
                  ) : (
                    <span className="text-[10px] px-1.5 py-0.5 rounded" style={{ color: "var(--warning)", background: "rgba(250,178,25,0.12)" }}>
                      需复核
                    </span>
                  )}
                </div>
              ))}
            </div>
            <div className="px-5 py-4 border-t border-[var(--grid)] flex items-center gap-3 justify-end">
              {busy && (
                <div className="mr-auto text-xs text-[var(--muted)]">处理中…</div>
              )}
              <button
                onClick={() => setConfirming(false)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
              >
                取消
              </button>
              <button
                onClick={() => doPrune(true)}
                disabled={busy}
                className="px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--accent)] hover:text-[var(--accent)] text-sm text-[var(--ink-2)]"
              >
                {busy ? "校验中…" : "先预演"}
              </button>
              <button
                onClick={() => doPrune(false)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-50 text-sm font-medium"
              >
                {busy ? "移除中…" : "确认重构"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
