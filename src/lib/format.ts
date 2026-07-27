export function fmtSize(bytes: number | null | undefined): string {
  if (bytes == null) return "…";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${bytes} B` : `${v.toFixed(1)} ${units[i]}`;
}

export function daysAgo(lastActiveMs: number | null | undefined): number | null {
  if (lastActiveMs == null) return null;
  return Math.floor((Date.now() - lastActiveMs) / 86_400_000);
}

export function fmtDaysAgo(lastActiveMs: number | null | undefined): string {
  const d = daysAgo(lastActiveMs);
  if (d == null) return "未知";
  if (d <= 0) return "今天";
  if (d < 30) return `${d} 天前`;
  if (d < 365) return `${Math.floor(d / 30)} 个月前`;
  return `${(d / 365).toFixed(1)} 年前`;
}

/** 毫秒时长格式化为人类可读：<1s 显示毫秒，否则 s，>=60s 显示 m:ss。 */
export function fmtDuration(ms: number | null | undefined): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${ms} ms`;
  const totalSec = ms / 1000;
  if (totalSec < 60) return `${totalSec.toFixed(totalSec < 10 ? 1 : 0)} s`;
  const m = Math.floor(totalSec / 60);
  const s = Math.round(totalSec % 60);
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}
