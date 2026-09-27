import { useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";

/**
 * "排除路径"设置面板：管理"扫描时永不触碰"的目录列表（持久化于 localStorage）。
 *
 * 与清理面板的 🚫 hover 按钮共用同一份 excludes 状态（App 持有，本面板受控）。
 * 这里提供：完整列表、目录选择器添加、手动粘贴添加、逐项删除、清空。
 */
export default function ExcludesPanel({
  excludes,
  onChange,
}: {
  excludes: string[];
  onChange: (next: string[]) => void;
}) {
  const { t } = useTranslation();
  const [manual, setManual] = useState("");
  const [error, setError] = useState<string | null>(null);

  function persist(next: string[]) {
    localStorage.setItem("excludes", JSON.stringify(next));
    onChange(next);
  }

  function add(dir: string) {
    const trimmed = dir.trim().replace(/\\/g, "/");
    if (!trimmed) return;
    if (excludes.some((e) => e.replace(/\\/g, "/") === trimmed)) {
      setError(t("excludes.dup"));
      return;
    }
    setError(null);
    persist([...excludes, trimmed]);
    setManual("");
  }

  async function pick() {
    const dir = await open({ directory: true });
    if (typeof dir === "string") add(dir);
  }

  function remove(dir: string) {
    persist(excludes.filter((e) => e !== dir));
  }

  function clearAll() {
    persist([]);
  }

  return (
    <main className="flex-1 overflow-auto px-6 pb-6">
      <div className="max-w-3xl">
        {/* 说明 */}
        <div className="mb-5">
          <h2 className="text-lg font-semibold text-[var(--ink-1)]">{t("excludes.title")}</h2>
          <p className="text-sm text-[var(--muted)] mt-1">
            {t("excludes.descPrefix")}
            <span className="text-[var(--ink-2)]">{t("excludes.descStrong")}</span>
            {t("excludes.descSuffix")}
          </p>
        </div>

        {/* 添加区 */}
        <div className="flex items-center gap-2 mb-4">
          <button
            onClick={pick}
            className="px-3 py-1.5 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] hover:border-[var(--baseline)] text-sm text-[var(--ink-2)]"
          >
            {t("excludes.pickDir")}
          </button>
          <input
            value={manual}
            onChange={(e) => setManual(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && add(manual)}
            placeholder={t("excludes.addPlaceholder")}
            className="flex-1 rounded-lg bg-[var(--surface)] border border-[var(--hairline)] focus:border-[var(--accent)] outline-none px-3 py-1.5 text-sm text-[var(--ink-2)] placeholder:text-[var(--muted)]"
          />
          <button
            onClick={() => add(manual)}
            disabled={!manual.trim()}
            className="px-3 py-1.5 rounded-lg bg-[var(--accent)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium"
          >
            {t("excludes.add")}
          </button>
        </div>
        {error && <div className="text-xs text-[var(--critical)] mb-3">{error}</div>}

        {/* 列表 */}
        <div className="rounded-xl border border-[var(--hairline)] bg-[var(--surface)] overflow-hidden min-h-[200px]">
          {excludes.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center gap-2 text-[var(--muted)] py-20">
              <span className="text-3xl">🛡️</span>
              <span className="text-sm">{t("excludes.empty")}</span>
            </div>
          ) : (
            <>
              <div className="flex items-center gap-3 px-4 py-2 border-b border-[var(--grid)] text-xs text-[var(--muted)]">
                <span>{t("excludes.countN", { count: excludes.length })}</span>
                <div className="flex-1" />
                <button
                  onClick={clearAll}
                  className="hover:text-[var(--critical)] underline"
                >
                  {t("excludes.clearAll")}
                </button>
              </div>
              {excludes.map((ex) => {
                const name = ex.split("/").filter(Boolean).pop() ?? ex;
                return (
                  <div
                    key={ex}
                    className="group flex items-center gap-3 px-4 py-2.5 border-b border-[var(--grid)] last:border-0 hover:bg-white/[0.03]"
                  >
                    <span className="text-lg shrink-0">📁</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-sm text-[var(--ink-1)] truncate">{name}</div>
                      <div className="text-xs text-[var(--muted)] truncate" title={ex}>
                        {ex}
                      </div>
                    </div>
                    <button
                      onClick={() => remove(ex)}
                      title={t("excludes.remove")}
                      className="opacity-0 group-hover:opacity-100 text-[var(--muted)] hover:text-[var(--critical)] transition-opacity text-sm"
                    >
                      ✕
                    </button>
                  </div>
                );
              })}
            </>
          )}
        </div>

        <p className="text-xs text-[var(--muted)] mt-3">
          {t("excludes.hintPrefix")}{" "}
          <code className="text-[var(--ink-2)]">D:/important</code>
          {t("excludes.hintSuffix")}
        </p>
      </div>
    </main>
  );
}
