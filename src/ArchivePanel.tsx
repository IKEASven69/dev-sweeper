import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { fmtSize } from "./lib/format";

interface ArchivableProject {
  name: string;
  path: string;
  sizeBytes: number;
  lastActiveMs: number | null;
  isGit: boolean;
}
interface ArchiveReport {
  name: string;
  sourcePath: string;
  archiveFile: string;
  originalSize: number;
  compressedSize: number;
  freedBytes: number;
  removedOriginal: boolean;
  error: string | null;
  dryRun: boolean;
  count?: number;
}
interface ArchiveFile {
  name: string;
  path: string;
  sizeBytes: number;
  projectName: string;
  createdAt: string;
}
interface RestoreReport {
  archiveFile: string;
  restoredTo: string;
  restoredBytes: number;
  error: string | null;
  dryRun: boolean;
}

/** "压缩归档"面板：把沉睡项目整体压成 .tar.gz，原项目移入回收站——源码不丢、随时还原。 */
export default function ArchivePanel({ projectDir }: { projectDir: string }) {
  const [projects, setProjects] = useState<ArchivableProject[]>([]);
  const [archives, setArchives] = useState<ArchiveFile[]>([]);
  const [archiveDir, setArchiveDir] = useState("");
  const [staleDays, setStaleDays] = useState(90);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [archiveResult, setArchiveResult] = useState<ArchiveReport | null>(null);
  const [restoreResult, setRestoreResult] = useState<RestoreReport | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    void discover();
    void refreshArchives();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectDir, staleDays]);

  useEffect(() => {
    if (!archiveResult) return;
    const t = setTimeout(() => setArchiveResult(null), 8000);
    return () => clearTimeout(t);
  }, [archiveResult]);
  useEffect(() => {
    if (!restoreResult) return;
    const t = setTimeout(() => setRestoreResult(null), 6000);
    return () => clearTimeout(t);
  }, [restoreResult]);

  async function discover() {
    if (!projectDir) return;
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<ArchivableProject[]>("discover_archivable", {
        root: projectDir,
        staleDays,
      });
      setProjects(r);
    } catch (e) {
      setError(String(e));
      setProjects([]);
    } finally {
      setLoading(false);
    }
  }

  async function refreshArchives() {
    try {
      const r = await invoke<ArchiveFile[]>("list_archives", { archiveDir });
      setArchives(r);
    } catch (e) {
      console.error(e);
    }
  }

  const totalBytes = useMemo(
    () => projects.reduce((s, p) => s + p.sizeBytes, 0),
    [projects],
  );
  const selectedBytes = useMemo(
    () =>
      projects
        .filter((p) => selected.has(p.path))
        .reduce((s, p) => s + p.sizeBytes, 0),
    [projects, selected],
  );

  async function doArchive(dry: boolean) {
    if (selected.size === 0) return;
    const n = selected.size;
    setBusy(true);
    let freed = 0;
    let compressed = 0;
    let failed = 0;
    const removed: string[] = [];
    try {
      for (const path of selected) {
        const r = await invoke<ArchiveReport>("archive_project", {
          dir: path,
          archiveDir,
          dryRun: dry,
        });
        if (r.error && !r.removedOriginal) {
          failed++;
        } else {
          freed += r.freedBytes;
          compressed += r.compressedSize;
          if (!dry) removed.push(path);
        }
      }
      setArchiveResult({
        name: "",
        sourcePath: "",
        archiveFile: "",
        originalSize: freed,
        compressedSize: compressed,
        freedBytes: freed,
        removedOriginal: removed.length > 0,
        error: failed ? `${failed} 项失败` : null,
        dryRun: dry,
        count: n,
      });
      if (!dry) {
        setProjects((prev) => prev.filter((p) => !removed.includes(p.path)));
        setSelected(new Set());
        void refreshArchives();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  }

  async function doRestore(file: string) {
    if (!projectDir) {
      setError("请先在顶栏选择项目根目录（作为还原目标）");
      return;
    }
    setBusy(true);
    try {
      const r = await invoke<RestoreReport>("restore_archive", {
        file,
        destRoot: projectDir,
        dryRun: false,
      });
      setRestoreResult(r);
    } catch (e) {
      setRestoreResult({
        archiveFile: file,
        restoredTo: "",
        restoredBytes: 0,
        error: String(e),
        dryRun: false,
      });
    } finally {
      setBusy(false);
    }
  }

  function toggle(path: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }
  function toggleAll() {
    setSelected(
      selected.size === projects.length
        ? new Set()
        : new Set(projects.map((p) => p.path)),
    );
  }

  return (
    <div className="flex-1 overflow-auto px-6 pb-6 space-y-5">
      {/* 说明 */}
      <div className="rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3 text-xs text-[var(--muted)]">
        把<strong className="text-[var(--ink-2)]">沉睡的项目</strong>整体压成{" "}
        <code>.tar.gz</code>{" "}
        归档，原项目移入回收站——源码不丢、需要时用「还原」解回。这是「不删也能瘦」的核心：活的项目的代码留着，只是不再占工作区的即时空间。
      </div>

      {/* 工具条 */}
      <div className="flex items-center gap-3 flex-wrap">
        <div className="text-sm text-[var(--ink-2)]">
          沉睡项目
          <span className="text-[var(--muted)] ml-1.5">
            {projects.length
              ? `· ${projectDir || "未选目录"} · 共 ${fmtSize(totalBytes)}`
              : projectDir
                ? "· 无"
                : "· 请先选择项目根目录"}
          </span>
        </div>
        <div className="flex-1" />
        <div className="flex items-center gap-1.5 text-xs text-[var(--muted)]">
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
        <button
          onClick={() => void discover()}
          disabled={loading || !projectDir}
          className="px-3 py-1.5 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)] disabled:opacity-40"
        >
          {loading ? "扫描中…" : "重新扫描"}
        </button>
      </div>

      {error && (
        <div className="px-1 py-2 text-sm" style={{ color: "var(--critical)" }}>
          ✕ {error}
        </div>
      )}

      {/* 区域① 沉睡项目列表 */}
      {projectDir && (
        <div className="rounded-xl border border-[var(--hairline)] bg-[var(--surface)] overflow-hidden">
          {projects.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-2 text-[var(--muted)] py-16">
              <span className="text-3xl">😴</span>
              <span className="text-sm">
                超过 {staleDays} 天未动的沉睡项目都在这；目前没有或都还很新鲜
              </span>
            </div>
          ) : (
            <div className="p-4 space-y-3">
              <div className="flex items-center gap-3">
                <input
                  type="checkbox"
                  checked={selected.size === projects.length && projects.length > 0}
                  onChange={toggleAll}
                />
                <span className="text-xs text-[var(--muted)]">
                  全选（{projects.length} 个）
                  {selected.size > 0 && ` · 已选 ${selected.size}`}
                </span>
                <div className="flex-1" />
                <button
                  onClick={() => setConfirming(true)}
                  disabled={busy || selected.size === 0}
                  className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-40 text-sm font-medium"
                >
                  {selected.size > 0 ? `归档选中（${selected.size}）` : "归档全部"}
                  {selectedBytes > 0 ? ` · ${fmtSize(selectedBytes)}` : ""}
                </button>
              </div>
              <div className="rounded-lg border border-[var(--grid)] divide-y divide-[var(--grid)]">
                {projects.map((p) => {
                  const sel = selected.has(p.path);
                  return (
                    <div
                      key={p.path}
                      className={`flex items-start gap-3 px-3 py-2.5 cursor-pointer ${
                        sel ? "bg-white/[0.04]" : "hover:bg-white/[0.03]"
                      }`}
                      onClick={() => toggle(p.path)}
                    >
                      <input
                        type="checkbox"
                        checked={sel}
                        onChange={() => toggle(p.path)}
                        onClick={(e) => e.stopPropagation()}
                      />
                      <span className="inline-flex size-9 items-center justify-center rounded-lg bg-[var(--grid)]/50 text-xl shrink-0">
                        📦
                      </span>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="font-medium text-[var(--ink-1)] truncate">
                            {p.name}
                          </span>
                          {p.isGit && (
                            <span
                              className="text-[10px] px-1.5 py-0.5 rounded text-[var(--muted)]"
                              style={{ background: "rgba(255,255,255,0.05)" }}
                            >
                              git
                            </span>
                          )}
                          <div className="flex-1" />
                          <span className="tabular-nums text-[var(--ink-2)] shrink-0">
                            {fmtSize(p.sizeBytes)}
                          </span>
                        </div>
                        <div
                          className="text-xs text-[var(--muted)] mt-0.5 truncate"
                          title={p.path}
                        >
                          {p.path}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}

      {/* 区域② 归档库 */}
      <div className="rounded-xl border border-[var(--hairline)] bg-[var(--surface)] overflow-hidden">
        <div className="flex items-center gap-3 px-4 py-3 border-b border-[var(--grid)]">
          <div className="text-sm text-[var(--ink-2)]">
            归档库
            <span className="text-[var(--muted)] ml-1.5">
              {archives.length ? `· ${archives.length} 个归档` : "· 默认 ~/dev-archives"}
            </span>
          </div>
          <div className="flex-1" />
          <input
            value={archiveDir}
            onChange={(e) => setArchiveDir(e.target.value)}
            placeholder="归档库目录（留空=默认 ~/dev-archives）"
            className="w-[280px] rounded-md bg-[var(--surface)] border border-[var(--hairline)] focus:border-[var(--accent)] px-2 py-1 text-xs text-[var(--ink-2)] outline-none"
          />
          <button
            onClick={() => void refreshArchives()}
            className="px-3 py-1.5 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
          >
            刷新
          </button>
        </div>
        {archives.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-2 text-[var(--muted)] py-12">
            <span className="text-2xl">🗜️</span>
            <span className="text-sm">
              归档库为空——归档项目后会在这里出现，可一键还原
            </span>
          </div>
        ) : (
          <div className="divide-y divide-[var(--grid)]">
            {archives.map((a) => (
              <div key={a.path} className="flex items-center gap-3 px-4 py-2.5">
                <span className="text-xl">🗜️</span>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-[var(--ink-1)] truncate">
                      {a.projectName}
                    </span>
                    {a.createdAt && (
                      <span className="text-[11px] text-[var(--muted)]">
                        归档于 {a.createdAt}
                      </span>
                    )}
                    <div className="flex-1" />
                    <span className="tabular-nums text-[var(--ink-2)] shrink-0">
                      {fmtSize(a.sizeBytes)}
                    </span>
                  </div>
                  <div
                    className="text-xs text-[var(--muted)] mt-0.5 truncate"
                    title={a.path}
                  >
                    {a.path}
                  </div>
                </div>
                <button
                  onClick={() => void doRestore(a.path)}
                  disabled={busy || !projectDir}
                  title={
                    projectDir
                      ? `解回到 ${projectDir}`
                      : "请先选择项目根目录作为还原目标"
                  }
                  className="px-3 py-1.5 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] hover:border-[var(--accent)] hover:text-[var(--accent)] text-sm text-[var(--ink-2)] disabled:opacity-40"
                >
                  还原
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* 归档结果 toast */}
      {archiveResult && (
        <div className="toast-in fixed bottom-5 right-5 z-20 rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3 text-sm shadow-xl max-w-md">
          {archiveResult.dryRun ? (
            <>
              🔍 预演：会归档 {archiveResult.count ?? 0} 个项目，释放{" "}
              {fmtSize(archiveResult.freedBytes)}，实际未执行
            </>
          ) : (
            <>
              <span style={{ color: "var(--good)" }}>✓</span> 已归档{" "}
              {archiveResult.count ?? 0} 个项目，压缩后{" "}
              {fmtSize(archiveResult.compressedSize)}，原项目已移入回收站
              {archiveResult.error && (
                <div className="mt-1 text-xs" style={{ color: "var(--critical)" }}>
                  ✕ {archiveResult.error}
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* 还原结果 toast */}
      {restoreResult && (
        <div className="toast-in fixed bottom-5 right-5 z-20 rounded-xl border border-[var(--hairline)] bg-[var(--surface)] px-4 py-3 text-sm shadow-xl max-w-md">
          {restoreResult.error ? (
            <>✕ 还原失败：{restoreResult.error}</>
          ) : (
            <>
              ↩️ 已解回 {restoreResult.restoredTo}（{fmtSize(restoreResult.restoredBytes)}）
            </>
          )}
        </div>
      )}

      {/* 确认对话框 */}
      {confirming && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center p-8 z-10">
          <div className="bg-[var(--surface)] border border-[var(--hairline)] rounded-2xl max-w-2xl w-full max-h-[80vh] flex flex-col shadow-2xl">
            <div className="px-5 py-4 border-b border-[var(--grid)]">
              <h2 className="font-semibold">
                归档 {selected.size} 个项目
                <span className="ml-2 text-[var(--ink-2)] font-normal">
                  {fmtSize(selectedBytes)}
                </span>
              </h2>
              <p className="text-sm text-[var(--muted)] mt-1">
                每个项目会被压成 .tar.gz（默认 ~/dev-archives），原项目移入回收站。源码不丢，随时可还原。清空回收站即真正释放空间。
              </p>
            </div>
            <div className="flex-1 overflow-auto px-5 py-3 text-sm space-y-2">
              {projects
                .filter((p) => selected.has(p.path))
                .map((p) => (
                  <div key={p.path} className="flex items-center gap-3">
                    <span className="text-xl">📦</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-[var(--ink-1)] truncate">{p.name}</div>
                      <div
                        className="text-xs text-[var(--muted)] truncate"
                        title={p.path}
                      >
                        {p.path}
                      </div>
                    </div>
                    <span className="text-[var(--muted)] text-xs whitespace-nowrap shrink-0">
                      {fmtSize(p.sizeBytes)}
                    </span>
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
                onClick={() => doArchive(true)}
                disabled={busy}
                className="px-3 py-1.5 rounded-lg bg-transparent border border-[var(--hairline)] hover:border-[var(--accent)] hover:text-[var(--accent)] text-sm text-[var(--ink-2)]"
              >
                {busy ? "校验中…" : "先预演"}
              </button>
              <button
                onClick={() => doArchive(false)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-[var(--critical)] hover:brightness-110 disabled:opacity-50 text-sm font-medium"
              >
                {busy ? "归档中…" : "确认归档"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
