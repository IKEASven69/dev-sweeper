import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { daysAgo, fmtDaysAgo, fmtSize } from "./lib/format";

interface Artifact {
  id: number;
  ruleId: string;
  path: string;
  projectDir: string;
  projectName: string;
  sizeBytes: number | null;
  lastActiveMs: number | null;
  regenHint: string;
}

interface DeleteReport {
  deleted: string[];
  failed: [string, string][];
}

/* 分类色按固定槽位顺序分配（dataviz 暗面校验通过的顺序），标签文字不沾系列色 */
const RULE_META: Record<string, { label: string; color: string; icon: string }> = {
  node: { label: "Node modules", color: "var(--s-node)", icon: "📦" },
  rust: { label: "Rust target", color: "var(--s-rust)", icon: "🦀" },
  maven: { label: "Maven target", color: "var(--s-maven)", icon: "☕" },
  gradle: { label: "Gradle build", color: "var(--s-gradle)", icon: "🐘" },
  "python-venv": { label: "Python venv", color: "var(--s-python-venv)", icon: "🐍" },
  "python-cache": { label: "Python cache", color: "var(--s-python-cache)", icon: "⚡" },
  "web-dist": { label: "Web dist", color: "var(--s-web-dist)", icon: "🌐" },
};

const ALL_RULES = Object.keys(RULE_META);
const DEFAULT_RULES = ALL_RULES.filter((r) => r !== "web-dist");

type SortMode = "size" | "stale";

/** 生态图标：大号 emoji 作为每行视觉锚点；未知 ruleId 回退为通用目录图标。 */
function EcoIcon({ ruleId, size = "md" }: { ruleId: string; size?: "md" | "lg" }) {
  const meta = RULE_META[ruleId];
  return (
    <span
      className={`inline-flex items-center justify-center rounded-lg bg-[var(--grid)]/50 ${
        size === "lg" ? "size-11 text-2xl" : "size-9 text-xl"
      }`}
    >
      {meta?.icon ?? "🗂️"}
    </span>
  );
}

function StatTile(props: { label: string; value: string; sub?: string; accent?: string }) {
  return (
    <div className="flex-1 rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3">
      <div className="text-xs text-[var(--muted)]">{props.label}</div>
      <div className="mt-1 text-2xl font-semibold" style={{ color: props.accent ?? "var(--ink-1)" }}>
        {props.value}
      </div>
      {props.sub && <div className="mt-0.5 text-xs text-[var(--ink-2)]">{props.sub}</div>}
    </div>
  );
}

export default function App() {
  const [root, setRoot] = useState<string>(() => localStorage.getItem("root") ?? "");
  const [ruleIds, setRuleIds] = useState<string[]>(DEFAULT_RULES);
  const [staleDays, setStaleDays] = useState(90);
  const [scanning, setScanning] = useState(false);
  const [scanProgress, setScanProgress] = useState<number | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [artifacts, setArtifacts] = useState<Artifact[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [sort, setSort] = useState<SortMode>("size");
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [lastReport, setLastReport] = useState<DeleteReport | null>(null);

  useEffect(() => {
    const subs = [
      listen<Artifact>("scan:found", (e) => {
        setArtifacts((prev) => [...prev, e.payload]);
      }),
      listen<{ id: number; size: number }>("scan:size", (e) => {
        setArtifacts((prev) =>
          prev.map((a) => (a.id === e.payload.id ? { ...a, sizeBytes: e.payload.size } : a)),
        );
      }),
      listen<{ scannedDirs: number }>("scan:progress", (e) => {
        setScanProgress(e.payload.scannedDirs);
      }),
      listen<{ cancelled: boolean }>("scan:done", (e) => {
        setScanning(false);
        setCancelled(e.payload.cancelled);
        setScanProgress(null);
      }),
      listen<{ done: number; total: number }>("delete:progress", (e) => {
        setProgress({ done: e.payload.done, total: e.payload.total });
      }),
    ];
    return () => {
      subs.forEach((s) => s.then((un) => un()));
    };
  }, []);

  useEffect(() => {
    if (!lastReport) return;
    const t = setTimeout(() => setLastReport(null), 6000);
    return () => clearTimeout(t);
  }, [lastReport]);

  async function pickDir() {
    const dir = await open({ directory: true, defaultPath: root || undefined });
    if (typeof dir === "string") {
      setRoot(dir);
      localStorage.setItem("root", dir);
    }
  }

  async function startScan() {
    if (!root || scanning) return;
    setArtifacts([]);
    setSelected(new Set());
    setLastReport(null);
    setCancelled(false);
    setScanProgress(null);
    setScanning(true);
    try {
      await invoke("scan", { root, ruleIds });
    } catch (e) {
      console.error(e);
      setScanning(false);
      setScanProgress(null);
    }
  }

  async function cancelScan() {
    try {
      await invoke("cancel_scan");
    } catch (e) {
      console.error(e);
    }
  }

  async function doDelete() {
    const items = artifacts.filter((a) => selected.has(a.id));
    setDeleting(true);
    setProgress({ done: 0, total: items.length });
    try {
      const report = await invoke<DeleteReport>("delete_artifacts", {
        paths: items.map((a) => a.path),
        dryRun: false,
      });
      const deletedSet = new Set(report.deleted);
      setArtifacts((prev) => prev.filter((a) => !deletedSet.has(a.path)));
      setSelected(new Set());
      setLastReport(report);
    } catch (e) {
      console.error(e);
    } finally {
      setDeleting(false);
      setConfirming(false);
      setProgress(null);
    }
  }

  const sorted = useMemo(() => {
    const copy = [...artifacts];
    if (sort === "size") {
      copy.sort((a, b) => (b.sizeBytes ?? -1) - (a.sizeBytes ?? -1));
    } else {
      copy.sort((a, b) => (a.lastActiveMs ?? Infinity) - (b.lastActiveMs ?? Infinity));
    }
    return copy;
  }, [artifacts, sort]);

  const totalBytes = useMemo(
    () => artifacts.reduce((s, a) => s + (a.sizeBytes ?? 0), 0),
    [artifacts],
  );
  const maxBytes = useMemo(
    () => Math.max(1, ...artifacts.map((a) => a.sizeBytes ?? 0)),
    [artifacts],
  );
  const selectedItems = artifacts.filter((a) => selected.has(a.id));
  const selectedBytes = selectedItems.reduce((s, a) => s + (a.sizeBytes ?? 0), 0);
  const isStale = (a: Artifact) => (daysAgo(a.lastActiveMs) ?? 0) >= staleDays;
  const staleItems = artifacts.filter(isStale);
  const staleBytes = staleItems.reduce((s, a) => s + (a.sizeBytes ?? 0), 0);
  const ruleCounts = useMemo(() => {
    const m = new Map<string, number>();
    artifacts.forEach((a) => m.set(a.ruleId, (m.get(a.ruleId) ?? 0) + 1));
    return m;
  }, [artifacts]);

  function toggle(id: number) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function toggleAll() {
    setSelected(selected.size === artifacts.length ? new Set() : new Set(artifacts.map((a) => a.id)));
  }

  function toggleRule(id: string) {
    setRuleIds((prev) => (prev.includes(id) ? prev.filter((r) => r !== id) : [...prev, id]));
  }

  return (
    <div className="min-h-screen flex flex-col">
      {/* 顶栏 */}
      <header className="px-6 pt-5 pb-4 space-y-4">
        <div className="flex items-center gap-3">
          <h1 className="text-base font-semibold tracking-wide flex items-center gap-2">
            <span className="inline-flex size-7 items-center justify-center rounded-lg bg-[var(--surface)] border border-[var(--hairline)]">
              🧹
            </span>
            dev-sweeper
          </h1>
          <div className="flex-1" />
          <button
            onClick={pickDir}
            className="px-3 py-1.5 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
          >
            选择目录
          </button>
          <input
            value={root}
            onChange={(e) => setRoot(e.target.value)}
            onBlur={() => localStorage.setItem("root", root)}
            onKeyDown={(e) => e.key === "Enter" && startScan()}
            placeholder="或直接粘贴路径，回车扫描"
            className="w-[340px] rounded-lg bg-[var(--surface)] border border-[var(--hairline)] focus:border-[var(--accent)] outline-none px-3 py-1.5 text-sm text-[var(--ink-2)] placeholder:text-[var(--muted)]"
          />
          <button
            onClick={startScan}
            disabled={!root || scanning}
            className="px-4 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium inline-flex items-center gap-2"
          >
            {scanning && (
              <span className="size-3 rounded-full border-2 border-white/40 border-t-white animate-spin" />
            )}
            {scanning
              ? scanProgress != null
                ? `扫描中 · ${scanProgress.toLocaleString()} 目录`
                : "扫描中"
              : "扫描"}
          </button>
          {scanning && (
            <button
              onClick={cancelScan}
              className="px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--critical)] hover:text-[var(--critical)] text-sm text-[var(--ink-2)]"
            >
              取消
            </button>
          )}
        </div>

        {cancelled && (
          <div className="text-xs text-[var(--warning)] -mt-1">
            ⏱ 扫描已取消，显示的是已发现的部分结果（部分大小可能仍为 …，尚未算完）。
          </div>
        )}

        {/* 统计卡片 */}
        <div className="flex gap-3">
          <StatTile
            label="可回收空间"
            value={artifacts.length ? fmtSize(totalBytes) : "—"}
            sub={scanning ? `已发现 ${artifacts.length} 项…` : artifacts.length ? `${artifacts.length} 个产物目录` : "扫描后统计"}
          />
          <StatTile
            label={`陈旧项目（超 ${staleDays} 天未动）`}
            value={artifacts.length ? String(staleItems.length) : "—"}
            sub={staleItems.length ? `共 ${fmtSize(staleBytes)}，删了最不心疼` : undefined}
            accent={staleItems.length ? "var(--warning)" : undefined}
          />
          <StatTile
            label="已选中"
            value={selected.size ? fmtSize(selectedBytes) : "—"}
            sub={selected.size ? `${selected.size} 项待清理` : "勾选后可移入回收站"}
            accent={selected.size ? "var(--accent)" : undefined}
          />
        </div>

        {/* 生态过滤 + 阈值 */}
        <div className="flex items-center gap-2 text-sm flex-wrap">
          {ALL_RULES.map((r) => {
            const active = ruleIds.includes(r);
            const count = ruleCounts.get(r);
            return (
              <button
                key={r}
                onClick={() => toggleRule(r)}
                className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs border transition-colors ${
                  active
                    ? "border-[var(--baseline)] bg-[var(--surface)] text-[var(--ink-2)]"
                    : "border-transparent text-[var(--muted)] opacity-50"
                }`}
              >
                <span className="size-2 rounded-full" style={{ background: RULE_META[r].color }} />
                {RULE_META[r].label}
                {active && count != null && (
                  <span className="text-[var(--muted)] tabular-nums">{count}</span>
                )}
              </button>
            );
          })}
          <div className="flex-1" />
          <div className="flex items-center gap-2 text-xs text-[var(--muted)]">
            <span>陈旧阈值</span>
            <input
              type="number"
              min={1}
              max={3650}
              value={staleDays}
              onChange={(e) => {
                const n = Number(e.target.value);
                setStaleDays(n >= 1 && n <= 3650 ? n : 90);
              }}
              className="w-16 rounded-md bg-[var(--surface)] border border-[var(--hairline)] focus:border-[var(--accent)] px-2 py-0.5 text-center text-[var(--ink-2)] outline-none tabular-nums"
            />
            <span>天未动</span>
          </div>
        </div>
      </header>

      {/* 操作条 */}
      {artifacts.length > 0 && (
        <div className="flex items-center gap-3 px-6 pb-2 text-sm">
          {staleItems.length > 0 && (
            <button
              onClick={() => setSelected(new Set(staleItems.map((a) => a.id)))}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-[var(--hairline)] hover:border-[var(--warning)] hover:text-[var(--warning)] text-xs text-[var(--ink-2)] transition-colors"
              title={`勾选所有超过 ${staleDays} 天未活跃的产物`}
            >
              ⏱ 全选 {staleItems.length} 个陈旧项（{fmtSize(staleBytes)}）
            </button>
          )}
          <div className="flex-1" />
          {selected.size > 0 && (
            <button
              onClick={() => setConfirming(true)}
              className="px-3.5 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 text-sm font-medium"
            >
              移入回收站（{selected.size}）
            </button>
          )}
        </div>
      )}

      {/* 产物列表 */}
      <main className="flex-1 overflow-auto px-6 pb-6">
        <div className="rounded-xl border border-[var(--hairline)] bg-[var(--surface)] overflow-hidden min-h-[280px]">
          {sorted.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center gap-2 text-[var(--muted)] py-24">
              {scanning ? (
                <>
                  <span className="size-6 rounded-full border-2 border-[var(--grid)] border-t-[var(--accent)] animate-spin" />
                  正在扫描…
                </>
              ) : (
                <>
                  <span className="text-3xl">🗂️</span>
                  <span className="text-sm">选择目录并点击「扫描」，找出可回收的构建产物</span>
                </>
              )}
            </div>
          ) : (
            <>
              {/* 轻量工具条：全选 + 计数 + 排序 */}
              <div className="sticky top-0 z-10 flex items-center gap-3 px-4 py-2 border-b border-[var(--grid)] bg-[var(--surface)] text-xs text-[var(--muted)]">
                <input
                  type="checkbox"
                  checked={selected.size === artifacts.length && artifacts.length > 0}
                  onChange={toggleAll}
                />
                <span>
                  {artifacts.length} 个产物{selected.size > 0 && ` · 已选 ${selected.size}`}
                </span>
                <div className="flex-1" />
                <button
                  onClick={() => setSort("size")}
                  className={`px-2 py-0.5 rounded hover:text-[var(--ink-2)] ${
                    sort === "size" ? "text-[var(--ink-2)] bg-[var(--grid)]/40" : ""
                  }`}
                >
                  按大小 {sort === "size" && "▾"}
                </button>
                <button
                  onClick={() => setSort("stale")}
                  className={`px-2 py-0.5 rounded hover:text-[var(--ink-2)] ${
                    sort === "stale" ? "text-[var(--ink-2)] bg-[var(--grid)]/40" : ""
                  }`}
                >
                  按活跃度 {sort === "stale" && "▾"}
                </button>
              </div>

              {/* 行列表 */}
              <div>
                {sorted.map((a) => {
                  const meta = RULE_META[a.ruleId];
                  const sel = selected.has(a.id);
                  return (
                    <div
                      key={a.id}
                      className={`group flex items-start gap-3 px-4 py-2.5 border-b border-[var(--grid)] last:border-0 hover:bg-white/[0.03] cursor-pointer ${
                        sel ? "bg-white/[0.04]" : ""
                      }`}
                      style={sel ? { borderLeft: `2px solid ${meta?.color ?? "var(--accent)"}` } : undefined}
                      onClick={() => toggle(a.id)}
                    >
                      {/* checkbox */}
                      <div className="pt-2 shrink-0" onClick={(e) => e.stopPropagation()}>
                        <input
                          type="checkbox"
                          checked={sel}
                          onChange={() => toggle(a.id)}
                        />
                      </div>
                      {/* 生态图标 */}
                      <div className="shrink-0 pt-0.5">
                        <EcoIcon ruleId={a.ruleId} />
                      </div>
                      {/* 主体 */}
                      <div className="flex-1 min-w-0">
                        {/* 主行 */}
                        <div className="flex items-center gap-2.5">
                          <span className="font-medium truncate text-[var(--ink-1)]">
                            {a.projectName}
                          </span>
                          <span className="text-[11px] text-[var(--muted)] shrink-0">
                            {meta?.label ?? a.ruleId}
                          </span>
                          {isStale(a) && (
                            <span
                              className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] shrink-0"
                              style={{ color: "var(--warning)", background: "rgba(250,178,25,0.12)" }}
                            >
                              ⏱ 陈旧
                            </span>
                          )}
                          <div className="flex-1" />
                          <span className="tabular-nums text-[var(--ink-2)] shrink-0">
                            {fmtSize(a.sizeBytes)}
                          </span>
                          <span className="text-[var(--muted)] text-xs whitespace-nowrap shrink-0">
                            {fmtDaysAgo(a.lastActiveMs)}
                          </span>
                        </div>
                        {/* 大小条 */}
                        <div className="mt-1.5 h-1.5 rounded-full bg-[var(--grid)] overflow-hidden">
                          <span
                            className="block h-full rounded-full transition-[width] duration-300"
                            style={{
                              width: `${Math.max(2, ((a.sizeBytes ?? 0) / maxBytes) * 100)}%`,
                              background: meta?.color ?? "var(--muted)",
                            }}
                          />
                        </div>
                        {/* 次行：路径 + 再生提示 */}
                        <div className="flex items-center gap-2 text-xs text-[var(--muted)] mt-1">
                          <span className="truncate" title={a.path} onClick={(e) => e.stopPropagation()}>
                            {a.path}
                          </span>
                          <span className="text-[var(--grid)] shrink-0">·</span>
                          <span className="shrink-0">{a.regenHint}</span>
                        </div>
                      </div>
                      {/* hover 操作 */}
                      <div className="pt-1.5 shrink-0" onClick={(e) => e.stopPropagation()}>
                        <button
                          onClick={() => revealItemInDir(a.path)}
                          title="在资源管理器中打开"
                          className="opacity-0 group-hover:opacity-100 text-[var(--muted)] hover:text-[var(--ink-1)] transition-opacity"
                        >
                          📂
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            </>
          )}
        </div>
      </main>

      {/* 删除结果 toast */}
      {lastReport && (
        <div className="toast-in fixed bottom-5 right-5 z-20 rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3 text-sm shadow-xl">
          <span style={{ color: "var(--good)" }}>✓</span> 已移入回收站{" "}
          {lastReport.deleted.length} 项，可随时恢复
          {lastReport.failed.length > 0 && (
            <div className="mt-1 text-xs" style={{ color: "var(--critical)" }}>
              ✕ {lastReport.failed.length} 项失败：{lastReport.failed[0][1]}
            </div>
          )}
        </div>
      )}

      {/* 确认对话框 */}
      {confirming && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center p-8 z-10">
          <div className="bg-[var(--surface)] border border-[var(--hairline)] rounded-2xl max-w-2xl w-full max-h-[80vh] flex flex-col shadow-2xl">
            <div className="px-5 py-4 border-b border-[var(--grid)]">
              <h2 className="font-semibold">
                将 {selectedItems.length} 个目录移入回收站
                <span className="ml-2 text-[var(--ink-2)] font-normal">{fmtSize(selectedBytes)}</span>
              </h2>
              <p className="text-sm text-[var(--muted)] mt-1">
                删除后可在回收站恢复；需要时可按提示重新生成。
              </p>
            </div>
            <div className="flex-1 overflow-auto px-5 py-3 text-sm space-y-2">
              {selectedItems.map((a) => (
                <div key={a.id} className="flex items-center gap-3">
                  <EcoIcon ruleId={a.ruleId} />
                  <div className="flex-1 min-w-0">
                    <div className="text-[var(--ink-1)] truncate" title={a.path}>
                      {a.projectName}
                    </div>
                    <div className="text-xs text-[var(--muted)] truncate" title={a.path}>
                      {a.path}
                    </div>
                  </div>
                  <span className="text-[var(--muted)] text-xs whitespace-nowrap shrink-0">
                    {a.regenHint}
                  </span>
                </div>
              ))}
            </div>
            <div className="px-5 py-4 border-t border-[var(--grid)] flex items-center gap-3 justify-end">
              {deleting && progress && (
                <div className="mr-auto flex items-center gap-2 text-xs text-[var(--muted)] w-48">
                  <span className="flex-1 h-1 rounded-full bg-[var(--grid)] overflow-hidden">
                    <span
                      className="block h-full rounded-full bg-[var(--accent)] transition-[width]"
                      style={{ width: `${(progress.done / progress.total) * 100}%` }}
                    />
                  </span>
                  {progress.done}/{progress.total}
                </div>
              )}
              <button
                onClick={() => setConfirming(false)}
                disabled={deleting}
                className="px-4 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
              >
                取消
              </button>
              <button
                onClick={doDelete}
                disabled={deleting}
                className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-50 text-sm font-medium"
              >
                {deleting ? "删除中…" : "确认移入回收站"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
