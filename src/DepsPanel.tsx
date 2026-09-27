import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { fmtSize } from "./lib/format";
import { getLogDecisions, setLogDecisions as persistLogDecisions } from "./lib/settings";

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
  const { t } = useTranslation();
  const [report, setReport] = useState<DepReport | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  // 分析请求序号：防并发分析时旧响应覆盖新响应
  const analyzeSeqRef = useRef(0);
  const [result, setResult] = useState<PruneReport | null>(null);
  const [confirming, setConfirming] = useState(false);
  // 决策日志开关（opt-in，localStorage 持久化，与清理面板共用同一设置）
  const [logDecisions, setLogDecisions] = useState<boolean>(() => getLogDecisions());

  // pnpm 迁移相关状态
  const [migrating, setMigrating] = useState(false);
  const [migrateResult, setMigrateResult] = useState<MigrateReport | null>(null);
  const [migrateConfirming, setMigrateConfirming] = useState(false);
  // 迁移子进程输出尾部（migrate:log 逐行转发来；80ms 批量应用，只留尾部若干行）
  const [migrateLog, setMigrateLog] = useState<string[]>([]);
  const migrateLogBufRef = useRef<string[]>([]);
  const migrateLogTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  function flushMigrateLog() {
    if (migrateLogTimerRef.current != null) {
      clearTimeout(migrateLogTimerRef.current);
      migrateLogTimerRef.current = null;
    }
    const buf = migrateLogBufRef.current;
    if (buf.length === 0) return;
    migrateLogBufRef.current = [];
    setMigrateLog((prev) => [...prev, ...buf].slice(-12));
  }

  // 迁移输出转发：pnpm/uv 子进程逐行 → migrate:log 事件 → 尾部展示
  useEffect(() => {
    const sub = listen<{ line: string }>("migrate:log", (e) => {
      migrateLogBufRef.current.push(e.payload.line);
      if (migrateLogTimerRef.current == null) {
        migrateLogTimerRef.current = setTimeout(flushMigrateLog, 80);
      }
    });
    return () => {
      void sub.then((un) => un());
      if (migrateLogTimerRef.current != null) clearTimeout(migrateLogTimerRef.current);
    };
  }, []);

  // 项目目录变化时清空旧结果
  useEffect(() => {
    setReport(null);
    setResult(null);
    setError(null);
    setSelected(new Set());
    setMigrateResult(null);
    setMigrateConfirming(false);
    setMigrating(false);
    setMigrateLog([]);
    migrateLogBufRef.current = [];
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
    // 请求序号守卫：顶栏"分析依赖"事件不走按钮禁用逻辑，双击会并发两次
    // invoke，先发后至的旧响应会覆盖新结果——只认最后一次请求的结果
    const seq = ++analyzeSeqRef.current;
    setAnalyzing(true);
    setError(null);
    setResult(null);
    setSelected(new Set());
    try {
      const r = await invoke<DepReport>("analyze_deps", { projectDir });
      if (seq !== analyzeSeqRef.current) return;
      setReport(r);
    } catch (e) {
      if (seq !== analyzeSeqRef.current) return;
      setError(String(e));
      setReport(null);
    } finally {
      if (seq === analyzeSeqRef.current) setAnalyzing(false);
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
        logDecisions: !dry && logDecisions, // 预演不是决策，不记日志
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
    setMigrateLog([]);
    migrateLogBufRef.current = [];
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

  /** 取消进行中的迁移：kill pnpm/uv 子进程，已完成步骤不回滚（旧目录在回收站可恢复）。 */
  async function cancelMigrate() {
    try {
      await invoke("cancel_migrate");
    } catch (e) {
      console.error(e);
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
            {t("deps.pickDir")}
          </button>
          <span className="text-xs text-[var(--muted)] truncate" title={projectDir}>
            {projectDir || t("deps.noDir")}
          </span>
          <div className="flex-1" />
          <button
            onClick={analyze}
            disabled={!projectDir || analyzing}
            className="px-4 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium"
          >
            {analyzing ? t("deps.analyzing") : t("deps.analyze")}
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
            <span className="text-sm">{t("deps.emptyTitle")}</span>
            <span className="text-xs">{t("deps.emptySub")}</span>
          </div>
        )}

        {report && (
          <div className="p-4 space-y-4">
            <div className="flex items-center gap-3 text-sm">
              <div className="flex-1 text-[var(--ink-2)]">
                {t("deps.summaryPrefix")}{" "}
                <span className="font-medium text-[var(--ink-1)]">{report.projectName}</span> ·{" "}
                {t("deps.summaryDecl")} <b>{report.declaredCount}</b>{" "}
                {t("deps.summaryDeclaredUnit")} {t("deps.summaryUsedPrefix")}{" "}
                <b>{report.usedCount}</b> {t("deps.summaryUsedUnit")}
              </div>
              {report.unused.length > 0 && (
                <button
                  onClick={toggleAll}
                  className="px-2.5 py-1 rounded-md border border-[var(--hairline)] hover:border-[var(--baseline)] text-xs text-[var(--ink-2)]"
                >
                  {allSelected ? t("deps.deselectAll") : t("deps.selectAllUnused")}
                </button>
              )}
            </div>

            {/* pnpm 迁移建议 */}
            {report.pm === "npm" || report.pm === "yarn" ? (
              <div className="rounded-lg border border-[var(--hairline)] px-3 py-2.5 flex items-center gap-3">
                <div className="flex-1 min-w-0">
                  <div className="text-sm text-[var(--ink-2)]">
                    {t("deps.pmCurrent")}
                    <span className="font-medium text-[var(--ink-1)]">
                      {report.pm === "npm" ? "npm" : "yarn"}
                    </span>
                  </div>
                  <div className="text-xs text-[var(--muted)] mt-0.5">
                    {t("deps.migrateHint")}
                  </div>
                </div>
                <button
                  onClick={() => setMigrateConfirming(true)}
                  disabled={migrating}
                  className="px-3 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-40 text-sm font-medium shrink-0"
                >
                  {t("deps.migrateTo")}
                </button>
              </div>
            ) : report.pm === "pnpm" ? (
              <div className="rounded-lg border border-[var(--hairline)] px-3 py-2.5 text-sm text-[var(--ink-2)]">
                {t("deps.alreadyPnpmPrefix")}{" "}
                <span className="font-medium text-[var(--ink-1)]">pnpm</span>
                {t("deps.alreadyPnpmSuffix")}
              </div>
            ) : null}

            {/* 未使用依赖 */}
            {report.unused.length === 0 ? (
              <div className="text-sm text-[var(--muted)] py-6 text-center">
                {t("deps.noUnused")}
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
                            {d.kind === "runtime" ? t("deps.kindRuntime") : t("deps.kindDev")}
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
                          {t("deps.confHigh")}
                        </span>
                      ) : (
                        <span
                          className="text-[10px] px-1.5 py-0.5 rounded shrink-0"
                          style={{ color: "var(--warning)", background: "rgba(250,178,25,0.12)" }}
                        >
                          {t("deps.confReview")}
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
                  {t("deps.extraneousTitle", { count: report.extraneous.length })}
                </div>
                <div className="text-xs text-[var(--ink-2)] break-words">
                  {report.extraneous.map((d) => d.name).join("、")}
                </div>
                <div className="text-xs text-[var(--muted)] mt-1">
                  {t("deps.extraneousHintPrefix")} <code>npm prune</code> /{" "}
                  <code>pnpm prune</code> {t("deps.extraneousHintSuffix")}
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
                  {t("deps.pruneButton", { count: selected.size })}
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
              <span style={{ color: "var(--accent)" }}>🔍</span>{" "}
              {t("deps.pruneToastDry", { count: result.removed.length })}
              {result.failed.length > 0 && (
                <div className="mt-1 text-xs" style={{ color: "var(--critical)" }}>
                  {t("deps.failedItems", { count: result.failed.length, err: result.failed[0][1] })}
                </div>
              )}
            </>
          ) : (
            <>
              <span style={{ color: "var(--good)" }}>✓</span>{" "}
              {t("deps.pruneToastDone", { count: result.removed.length, size: fmtSize(result.freedBytes) })}
              {result.backupPath && (
                <div className="text-xs text-[var(--muted)] mt-0.5">
                  {t("deps.backup", { path: result.backupPath })}
                </div>
              )}
              {result.failed.length > 0 && (
                <div className="mt-1 text-xs" style={{ color: "var(--critical)" }}>
                  {t("deps.failedItems", { count: result.failed.length, err: result.failed[0][1] })}
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
              <span style={{ color: "var(--accent)" }}>🔍</span>{" "}
              {t("deps.migrateToastDry", {
                pm: migrateResult.fromPm === "npm" ? "npm" : "yarn",
              })}
            </>
          ) : migrateResult.reinstalled ? (
            <>
              <span style={{ color: "var(--good)" }}>✓</span>{" "}
              {t("deps.migrateToastDone", { size: fmtSize(migrateResult.freedBytes) })}
              {migrateResult.backupPath && (
                <div className="text-xs text-[var(--muted)] mt-0.5">
                  {t("deps.trashPath", { path: migrateResult.backupPath })}
                </div>
              )}
            </>
          ) : (
            <>
              <span style={{ color: "var(--critical)" }}>✕</span>{" "}
              {t("deps.migrateFailed", { err: migrateResult.error ?? t("deps.unknownError") })}
              <div className="text-xs text-[var(--muted)] mt-0.5">
                {t("deps.migrateRecover")}
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
              <h2 className="font-semibold">
                {t("deps.migrateTitle", { pm: report?.pm === "npm" ? "npm" : "yarn" })}
              </h2>
              <p className="text-sm text-[var(--muted)] mt-1">
                {t("deps.migrateDescPrefix")} <code>pnpm import</code> +{" "}
                <code>pnpm install</code> {t("deps.migrateDescSuffix")}
              </p>
            </div>
            {/* 迁移中：子进程输出尾部 + 取消按钮 */}
            {migrating && (
              <div className="px-5 py-3 border-b border-[var(--grid)]">
                <div className="text-xs text-[var(--muted)] mb-1.5 flex items-center gap-2">
                  <span className="size-3 rounded-full border-2 border-[var(--grid)] border-t-[var(--accent)] animate-spin" />
                  {t("deps.migrateLog", { count: migrateLog.length })}
                </div>
                <pre className="text-[11px] leading-4 text-[var(--ink-2)] max-h-36 overflow-auto bg-black/30 rounded-lg p-2.5 whitespace-pre-wrap break-all font-mono">
                  {migrateLog.length > 0 ? migrateLog.join("\n") : t("deps.waitingPnpm")}
                </pre>
              </div>
            )}
            <div className="px-5 py-4 border-t border-[var(--grid)] flex items-center gap-3 justify-end">
              {migrating && (
                <button
                  onClick={cancelMigrate}
                  className="mr-auto px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--critical)] hover:text-[var(--critical)] text-sm text-[var(--ink-2)]"
                  title={t("deps.cancelMigrateTitle")}
                >
                  {t("deps.cancelMigrate")}
                </button>
              )}
              <button
                onClick={() => setMigrateConfirming(false)}
                disabled={migrating}
                className="px-4 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => doMigrate(true)}
                disabled={migrating}
                className="px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--accent)] hover:text-[var(--accent)] text-sm text-[var(--ink-2)]"
              >
                {migrating ? t("common.previewBusy") : t("common.preview")}
              </button>
              <button
                onClick={() => doMigrate(false)}
                disabled={migrating}
                className="px-4 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-50 text-sm font-medium"
              >
                {migrating ? t("deps.migrating") : t("deps.confirmMigrate")}
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
                {t("deps.pruneTitle", { count: selectedItems.length })}
              </h2>
              <p className="text-sm text-[var(--muted)] mt-1">{t("deps.pruneDescPrefix")}</p>
            </div>
            <div className="flex-1 overflow-auto px-5 py-3 text-sm space-y-2">
              {selectedItems.map((d) => (
                <div key={d.name} className="flex items-center gap-3">
                  <span className="font-medium text-[var(--ink-1)]">{d.name}</span>
                  <span
                    className="text-[10px] px-1.5 py-0.5 rounded"
                    style={{ background: "rgba(255,255,255,0.05)", color: "var(--muted)" }}
                  >
                    {d.kind === "runtime" ? t("deps.kindRuntime") : t("deps.kindDev")}
                  </span>
                  {d.confidence === "high" ? (
                    <span className="text-[10px] px-1.5 py-0.5 rounded" style={{ color: "var(--good)", background: "rgba(60,200,120,0.12)" }}>
                      {t("deps.confHigh")}
                    </span>
                  ) : (
                    <span className="text-[10px] px-1.5 py-0.5 rounded" style={{ color: "var(--warning)", background: "rgba(250,178,25,0.12)" }}>
                      {t("deps.confReview")}
                    </span>
                  )}
                </div>
              ))}
              <label
                className="flex items-center gap-2 pt-2 mt-1 border-t border-[var(--grid)] text-xs text-[var(--muted)] cursor-pointer"
                title={t("deps.logPruneTitle")}
              >
                <input
                  type="checkbox"
                  checked={logDecisions}
                  disabled={busy}
                  onChange={(e) => {
                    setLogDecisions(e.target.checked);
                    persistLogDecisions(e.target.checked);
                  }}
                />
                {t("deps.logPrune")}
              </label>
            </div>
            <div className="px-5 py-4 border-t border-[var(--grid)] flex items-center gap-3 justify-end">
              {busy && (
                <div className="mr-auto text-xs text-[var(--muted)]">{t("common.processing")}</div>
              )}
              <button
                onClick={() => setConfirming(false)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => doPrune(true)}
                disabled={busy}
                className="px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--accent)] hover:text-[var(--accent)] text-sm text-[var(--ink-2)]"
              >
                {busy ? t("common.previewBusy") : t("common.preview")}
              </button>
              <button
                onClick={() => doPrune(false)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-50 text-sm font-medium"
              >
                {busy ? t("deps.removing") : t("deps.confirmPrune")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
