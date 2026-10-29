// 键盘面纪律自检（评审 P1-01 防回归）：模式化键盘面一律挂 window 捕获、
// 永不挂面板/容器 onKeyDown——焦点掉 body（React 根之外）后容器 onKeyDown
// 收不到任何键，这是 2026-10-22 表单 Esc / 2026-10-28 合并模式 /
// 2026-10-28 StageView / 2026-10-29 右键菜单四次同根因事故的来源。
// 本脚本把全仓 onKeyDown 的 JSX 出现点与白名单比对：新增出现点必须先判定
// 是「元素级」（input/textarea 自身的快捷键或 Enter 提交）并登记白名单，
// 容器级一律改走 window 捕获（useEscCapture / 独立 useEffect）。
// 白名单按「文件 + 行内容」匹配（行号漂移不敏感；同文件同内容行共享一条）。
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]):/, "$1:");
const SKIP_DIRS = new Set(["components", "__tests__", "node_modules", "dist"]);
// 元素级用法与唯一保留的容器级用法（主面板导航键，挂载时 focusPanelRef 保证
// 焦点在面板根；任何新模式化键盘面不得再上容器 onKeyDown）
const ALLOW = [
  ["src/App.tsx", "onKeyDown={onKeyDown}"],
  ["src/plugins/vault/VaultRevealOverlay.tsx", "onKeyDown={(e) => {"],
  ["src/views/SettingsView.tsx", "onKeyDown={(e) => {"],
];

function walk(dir, out) {
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.(tsx|ts)$/.test(name)) out.push(p);
  }
  return out;
}

const files = walk(join(ROOT, "src"), []);
const violations = [];
for (const f of files) {
  const rel = relative(ROOT, f).split(sep).join("/");
  readFileSync(f, "utf8").split("\n").forEach((line) => {
    if (!line.includes("onKeyDown={")) return;
    const content = line.trim();
    if (content.startsWith("//") || content.startsWith("*")) return; // 注释不算
    const ok = ALLOW.some(([file, c]) => file === rel && content.includes(c));
    if (!ok) violations.push(`${rel}: ${content}`);
  });
}

if (violations.length > 0) {
  console.error("键盘面纪律违规——新增的 onKeyDown 出现点未登记白名单：");
  for (const v of violations) console.error("  " + v);
  console.error(
    "模式化键盘面一律 window 捕获（useEscCapture / 独立 useEffect capture），\n" +
      "容器级 onKeyDown 在焦点掉 body 后收不到任何键（四次事故同根因）。\n" +
      "元素级用法（input/textarea 自身快捷键）确认后登记进 scripts/check-keyboard-faces.mjs 的 ALLOW。"
  );
  process.exit(1);
}
console.log("键盘面纪律自检通过：onKeyDown 出现点全部在白名单内");
