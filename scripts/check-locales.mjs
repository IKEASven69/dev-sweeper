// 校验 zh/en locale 文件：key 集合一致 + 插值占位符一致。
// 用途：CI 防一边加 key 另一边漏译、占位符变量漂移。
import { readFileSync } from "node:fs";

const load = (f) =>
  JSON.parse(readFileSync(new URL(`../src/locales/${f}.json`, import.meta.url), "utf8"));

const flat = (o, p = "") =>
  Object.entries(o).flatMap(([k, v]) =>
    typeof v === "object" && v !== null ? flat(v, `${p}${k}.`) : [[`${p}${k}`, v]]);

const zh = new Map(flat(load("zh")));
const en = new Map(flat(load("en")));
const ph = (s) =>
  [...String(s).matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort().join(",");

const missEn = [...zh.keys()].filter((k) => !en.has(k));
const missZh = [...en.keys()].filter((k) => !zh.has(k));
const badPh = [...zh.keys()]
  .filter((k) => en.has(k) && ph(zh.get(k)) !== ph(en.get(k)))
  .map((k) => `  ${k}: zh(${ph(zh.get(k))}) vs en(${ph(en.get(k))})`);

if (missEn.length || missZh.length || badPh.length) {
  if (missEn.length) console.error("en 缺 key:", missEn);
  if (missZh.length) console.error("zh 缺 key:", missZh);
  if (badPh.length) console.error("占位符不一致:\n" + badPh.join("\n"));
  process.exit(1);
}
console.log(`locales ok: ${zh.size} keys, placeholders aligned`);
