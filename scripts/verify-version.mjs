// 版本一致性校验（评审 P1-15）：四处版本（package.json / tauri.conf.json /
// Cargo.toml / Cargo.lock）必须相等——发版 bump 漏改任意一处会导致 tag 校验
// 失败或产物/更新清单版本漂移。接线 pre-push 钩子与 CI；发版 bump 后本地跑
// `node scripts/verify-version.mjs` 即可确认。
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");

const pkg = JSON.parse(read("package.json")).version;
const tauri = JSON.parse(read("src-tauri/tauri.conf.json")).version;
const toml = /^version\s*=\s*"([^"]+)"/m.exec(read("src-tauri/Cargo.toml"))?.[1];
// Cargo.lock 里 zap 包自己的版本（第一个 name = "zap" 段）
const lockText = read("src-tauri/Cargo.lock");
const lock = /^name = "zap"$/m.test(lockText)
  ? /^version = "([^"]+)"$/m.exec(lockText.slice(lockText.indexOf('name = "zap"')))?.[1]
  : undefined;

const versions = { "package.json": pkg, "tauri.conf.json": tauri, "Cargo.toml": toml, "Cargo.lock": lock };
const bad = Object.entries(versions).filter(([, v]) => !v);
if (bad.length > 0) {
  console.error(`版本解析失败：${bad.map(([k]) => k).join(", ")}`);
  process.exit(1);
}
const uniq = new Set(Object.values(versions));
if (uniq.size !== 1) {
  console.error("四处版本不一致（发版 bump 需四处同步）：");
  for (const [k, v] of Object.entries(versions)) console.error(`  ${k}: ${v}`);
  process.exit(1);
}
console.log(`版本一致性通过：${pkg}`);
