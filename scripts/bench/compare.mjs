// 当前 criterion 输出 vs 基线，超阈值判回归（scripts/bench 持续方案，2026-11-07）。
// 阈值刻意放宽（中位 +20%）：性能测量噪声大，宁可漏报不可误报——误报的门禁会被绕过。
// 发版清单勾选项：L2 无回归 = 本脚本 exit 0。
// 用法：pnpm bench && pnpm bench:compare。
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import os from 'node:os';

// 阈值由 A/A 实测定（同码两跑，2026-11-07）：亚微秒级基准的运行间噪声中位
// 波动达 ±22%（criterion 自带检测也因此误报回归/提升），+35% 留足余量——
// 真回归（2 倍级劣化）照样拦截，噪声不再误伤。低于此阈值宁漏报不误报。
const THRESHOLD = 1.35; // 中位 +35% 判回归

const baselinePath = join(process.cwd(), 'docs', 'benchmarks', 'baseline.json');
if (!existsSync(baselinePath)) {
  console.error('基线不存在，先 pnpm bench:baseline');
  process.exit(1);
}
const baseline = JSON.parse(readFileSync(baselinePath, 'utf8'));
if (baseline.machine.host !== os.hostname()) {
  console.warn(`⚠️ 基线来自 ${baseline.machine.host}，本机 ${os.hostname()}——数字不可比，仅作参考`);
}

const base = process.env.CARGO_TARGET_DIR || join(process.cwd(), 'src-tauri', 'target');
const criterionDir = [join(base, 'criterion'), join(base, 'release', 'criterion')].find((p) =>
  existsSync(p),
);
if (!criterionDir) {
  console.error('找不到 criterion 输出——先跑 pnpm bench');
  process.exit(1);
}

const current = {};
const walk = (dir) => {
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, name.name);
    if (name.isDirectory()) walk(full);
    else if (name.name === 'estimates.json' && dir.endsWith('new')) {
      const est = JSON.parse(readFileSync(full, 'utf8'));
      const id = relative(criterionDir, dir).replace(/[/\\]new$/, '').replace(/\\/g, '/');
      current[id] = Math.round(est.median.point_estimate);
    }
  }
};
walk(criterionDir);

let regressed = 0;
console.log('基准                                  基线(ns)   当前(ns)   变化    判定');
for (const [id, baseNs] of Object.entries(baseline.numbers)) {
  const now = current[id];
  if (now === undefined) {
    console.log(`${id.padEnd(36)} ${String(baseNs).padEnd(10)} 缺测（本轮未跑）`);
    continue;
  }
  const ratio = now / baseNs;
  const verdict = ratio > THRESHOLD ? '⛔ 回归' : '✅';
  if (ratio > THRESHOLD) regressed += 1;
  console.log(
    `${id.padEnd(36)} ${String(baseNs).padEnd(10)} ${String(now).padEnd(10)} ${((ratio - 1) * 100).toFixed(1).padStart(6)}%  ${verdict}`,
  );
}
for (const id of Object.keys(current)) {
  if (!baseline.numbers[id]) console.log(`${id.padEnd(36)} 基线缺项——p npm bench:baseline 重建后纳入`);
}
if (regressed > 0) {
  console.error(`\n${regressed} 项超阈值（中位 +${Math.round((THRESHOLD - 1) * 100)}%）——发版前必须处置`);
  process.exit(1);
}
console.log('\nL2 无回归 ✅');
