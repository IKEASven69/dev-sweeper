/**
 * GUI 本地设置（localStorage 持久化）。
 *
 * 目前只有一项：决策日志开关（opt-in）。开启后删除/裁剪决策会以 JSONL
 * 追加到 ~/.dev-sweeper/decisions.jsonl，供事后审计；默认关闭（不留痕）。
 */

const LOG_DECISIONS_KEY = "logDecisions";

export function getLogDecisions(): boolean {
  return localStorage.getItem(LOG_DECISIONS_KEY) === "1";
}

export function setLogDecisions(v: boolean) {
  localStorage.setItem(LOG_DECISIONS_KEY, v ? "1" : "0");
}
