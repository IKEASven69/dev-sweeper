import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { fmtSize } from "./lib/format";

type CacheEco = "node" | "rust" | "java" | "go" | "python" | "pnpm" | "unknown";

interface CacheEntry {
  id: string;
  eco: CacheEco;
  label: string;
  path: string;
  sizeBytes: number;
  regenHint: string;
  risk: "safe" | "notice" | string;
}

interface CachePurgeReport {
  id: string;
  path: string;
  freedBytes: number;
  reinstallable: boolean;
  error: string | null;
  dryRun: boolean;
}

const ECO_META: Record<string, { label: string; color: string; icon: string }> = {
  node: { label: "Node", color: "var(--s-node)", icon: "📦" },
  rust: { label: "Rust", color: "var(--s-rust)", icon: "🦀" },
  java: { label: "Java", color: "var(--s-maven)", icon: "☕" },
  go: { label: "Go", color: "var(--s-godot)", icon: "🐹" },
  python: { label: "Python", color: "var(--s-python-venv)", icon: "🐍" },
  pnpm: { label: "pnpm", color: "var(--accent)", icon: "🔗" },
  unknown: { label: "?", color: "var(--muted)", icon: "🗂️" },
};

/** "全局缓存"面板：列出各语言包管理器的全局共享缓存，可勾选后清理（移入回收站）。 */
export default function CachesPanel() {
  const [caches, setCaches] = useState<CacheEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<CachePurgeReport | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    void discover();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!result) return;
    const t = setTimeout(() => setResult(null), 8000);
    return () => clearTimeout(t);
  }, [result]);

  async function discover() {
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<CacheEntry[]>("discover_caches");
      setCaches(r);
    } catch (e) {
      setError(String(e));
      setCaches([]);
    } finally {
      setLoading(false);
    }
  }

  const totalBytes = useMemo(
    () => caches.reduce((s, c) => s + c.sizeBytes, 0),
    [caches],
  );
  const selectedBytes = useMemo(
    () => caches.filter((c) => selected.has(c.id)).reduce((s, c) => s + c.sizeBytes, 0),
    [caches, selected],
  );
  // 清理目标：有选中则清理选中，否则清理全部
  const targetIds = selected.size > 0 ? [...selected] : caches.map((c) => c.id);
  const targetBytes = selected.size > 0 ? selectedBytes : totalBytes;

  async function doPurge(dry: boolean) {
    if (targetIds.length === 0) return;
    setBusy(true);
    let freed = 0;
    const failed: [string, string][] = [];
    try {
      for (const id of targetIds) {
        const r = await invoke<CachePurgeReport>("purge_cache", { id, dryRun: dry });
        if (r.error) failed.push([id, r.error]);
        else freed += r.freedBytes;
      }
      setResult({
        id: "",
        path: "",
        freedBytes: freed,
        reinstallable: true,
        error: failed.length ? failed[0][1] : null,
        dryRun: dry,
      });
      if (!dry) {
        const removed = new Set(targetIds);
        setCaches((prev) => prev.filter((c) => !removed.has(c.id)));
        setSelected(new Set());
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  }

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleAll() {
    setSelected(selected.size === caches.length ? new Set() : new Set(caches.map((c) => c.id)));
  }

  return (
    <div className="flex-1 overflow-auto px-6 pb-6">
      <div className="rounded-xl border border-[var(--hairline)] bg-[var(--surface)] overflow-hidden min-h-[280px]">
        {/* 工具条 */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-[var(--grid)]">
          <div className="text-sm text-[var(--ink-2)]">
            全局依赖缓存
            <span className="text-[var(--muted)] ml-1.5">
              {caches.length ? `· 共 ${fmtSize(totalBytes)}` : ""}
            </span>
          </div>
          <div className="flex-1" />
          <button
            onClick={discover}
            disabled={loading}
            className="px-3 py-1.5 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)] disabled:opacity-40"
          >
            {loading ? "扫描中…" : "重新扫描"}
          </button>
        </div>

        <div className="px-4 py-2.5 text-xs text-[var(--muted)] border-b border-[var(--grid)]">
          这些是各语言包管理器的<strong className="text-[var(--ink-2)]">全局共享缓存</strong>（不在项目内，普通清理碰不到）：清了只是重新下载，不丢任何源码；统一移入回收站，随时可恢复。
        </div>

        {error && (
          <div className="px-4 py-3 text-sm" style={{ color: "var(--critical)" }}>
            ✕ {error}
          </div>
        )}

        {!loading && caches.length === 0 && !error && (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-[var(--muted)] py-24">
            <span className="text-3xl">🧊</span>
            <span className="text-sm">未发现全局依赖缓存（或本机尚未安装对应工具链）</span>
            <span className="text-xs">npm / pip / cargo / maven / gradle / go / uv / pnpm 的全局缓存在这里集中管理</span>
          </div>
        )}

        {caches.length > 0 && (
          <div className="p-4 space-y-3">
            {/* 全选 + 清理操作 */}
            <div className="flex items-center gap-3">
              <input
                type="checkbox"
                checked={selected.size === caches.length && caches.length > 0}
                onChange={toggleAll}
              />
              <span className="text-xs text-[var(--muted)]">
                全选（{caches.length} 个）
                {selected.size > 0 && ` · 已选 ${selected.size}`}
              </span>
              <div className="flex-1" />
              <button
                onClick={() => setConfirming(true)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium"
              >
                {selected.size > 0 ? `清理选中（${selected.size}）` : "清理全部"}
                {targetBytes > 0 ? ` · ${fmtSize(targetBytes)}` : ""}
              </button>
            </div>

            {/* 列表 */}
            <div className="rounded-lg border border-[var(--grid)] divide-y divide-[var(--grid)]">
              {caches.map((c) => {
                const sel = selected.has(c.id);
                const meta = ECO_META[c.eco] ?? ECO_META.unknown;
                return (
                  <div
                    key={c.id}
                    className={`flex items-start gap-3 px-3 py-2.5 cursor-pointer ${
                      sel ? "bg-white/[0.04]" : "hover:bg-white/[0.03]"
                    }`}
                    onClick={() => toggle(c.id)}
                  >
                    <input
                      type="checkbox"
                      checked={sel}
                      onChange={() => toggle(c.id)}
                      onClick={(e) => e.stopPropagation()}
                    />
                    <span
                      className="inline-flex size-9 items-center justify-center rounded-lg bg-[var(--grid)]/50 text-xl shrink-0"
                    >
                      {meta.icon}
                    </span>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="font-medium text-[var(--ink-1)] truncate">{c.label}</span>
                        <span className="text-[11px] text-[var(--muted)] shrink-0">{meta.label}</span>
                        {c.risk === "notice" ? (
                          <span
                            className="text-[10px] px-1.5 py-0.5 rounded shrink-0"
                            style={{ color: "var(--warning)", background: "rgba(250,178,25,0.12)" }}
                            title="清了会损失跨项目去重 / 需重新下载"
                          >
                            🟡 注意
                          </span>
                        ) : (
                          <span
                            className="text-[10px] px-1.5 py-0.5 rounded shrink-0 text-[var(--muted)]"
                            style={{ background: "rgba(255,255,255,0.05)" }}
                            title="纯下载缓存，随时重下"
                          >
                            🟢 安全
                          </span>
                        )}
                        <div className="flex-1" />
                        <span className="tabular-nums text-[var(--ink-2)] shrink-0">
                          {fmtSize(c.sizeBytes)}
                        </span>
                      </div>
                      <div className="text-xs text-[var(--muted)] mt-0.5 truncate" title={c.path}>
                        {c.path}
                      </div>
                      <div className="text-xs text-[var(--muted)] mt-0.5">{c.regenHint}</div>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>

      {/* 结果 toast */}
      {result && (
        <div className="toast-in fixed bottom-5 right-5 z-20 rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3 text-sm shadow-xl max-w-md">
          {result.dryRun ? (
            <>
              <span style={{ color: "var(--accent)" }}>🔍</span> 预演：本会清理 {targetIds.length} 个全局缓存，
              释放 {fmtSize(result.freedBytes)}，实际未删除
            </>
          ) : (
            <>
              <span style={{ color: "var(--good)" }}>✓</span> 已清理全局缓存，释放{" "}
              {fmtSize(result.freedBytes)}，可在回收站恢复
            </>
          )}
          {result.error && (
            <div className="mt-1 text-xs" style={{ color: "var(--critical)" }}>
              ✕ 部分失败：{result.error}
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
                清理 {targetIds.length} 个全局缓存
                <span className="ml-2 text-[var(--ink-2)] font-normal">{fmtSize(targetBytes)}</span>
              </h2>
              <p className="text-sm text-[var(--muted)] mt-1">
                这些缓存由包管理器共享、不在项目内，移入回收站后可随时恢复，需要时按提示重新生成即可。
              </p>
            </div>
            <div className="flex-1 overflow-auto px-5 py-3 text-sm space-y-2">
              {caches
                .filter((c) => targetIds.includes(c.id))
                .map((c) => (
                  <div key={c.id} className="flex items-center gap-3">
                    <span className="text-xl">{ECO_META[c.eco]?.icon ?? "🗂️"}</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-[var(--ink-1)] truncate">{c.label}</div>
                      <div className="text-xs text-[var(--muted)] truncate" title={c.path}>
                        {c.path}
                      </div>
                    </div>
                    <span className="text-[var(--muted)] text-xs whitespace-nowrap shrink-0">
                      {fmtSize(c.sizeBytes)}
                    </span>
                  </div>
                ))}
            </div>
            <div className="px-5 py-4 border-t border-[var(--grid)] flex items-center gap-3 justify-end">
              {busy && <div className="mr-auto text-xs text-[var(--muted)]">处理中…</div>}
              <button
                onClick={() => setConfirming(false)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
              >
                取消
              </button>
              <button
                onClick={() => doPurge(true)}
                disabled={busy}
                className="px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--accent)] hover:text-[var(--accent)] text-sm text-[var(--ink-2)]"
              >
                {busy ? "校验中…" : "先预演"}
              </button>
              <button
                onClick={() => doPurge(false)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-50 text-sm font-medium"
              >
                {busy ? "清理中…" : "确认清理"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
