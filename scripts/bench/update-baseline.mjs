// 从 Criterion 输出提取中位数 → 落 docs/benchmarks/baseline.json（scripts/bench 持续方案，2026-11-07）。
// 基线只对「同机」有效：比较在 compare.mjs 里按 machine 字段告警。数字单位 ns。
// 用法：先跑 pnpm bench，再 pnpm bench:baseline。更新基线是显式动作（confirm 档，发版说明可查）。
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import os from 'node:os';

const base = process.env.CARGO_TARGET_DIR || join(process.cwd(), 'src-tauri', 'target');
// criterion 写在 target 根（target/criterion），个别版本写进 profile 子目录——两处都探
const candidates = [join(base, 'criterion'), join(base, 'release', 'criterion')];
const criterionDir = candidates.find((p) => existsSync(p));
if (!criterionDir) {
  console.error(`找不到 criterion 输出：${candidates.join(' / ')}\n先跑 pnpm bench`);
  process.exit(1);
}

// 递归找 new/estimates.json；bench id = 相对 criterionDir 的目录路径去掉 new/estimates.json
const numbers = {};
const walk = (dir) => {
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, name.name);
    if (name.isDirectory()) walk(full);
    else if (name.name === 'estimates.json' && dir.endsWith('new')) {
      const est = JSON.parse(readFileSync(full, 'utf8'));
      // id 归一为 / 分隔：Windows relative() 给反斜杠，基线文件要跨机可读
      const id = relative(criterionDir, dir).replace(/[/\\]new$/, '').replace(/\\/g, '/');
      numbers[id] = Math.round(est.median.point_estimate);
    }
  }
};
walk(criterionDir);
if (Object.keys(numbers).length === 0) {
  console.error('criterion 输出为空——先跑 pnpm bench');
  process.exit(1);
}

const baseline = {
  machine: {
    host: os.hostname(),
    cpu: os.cpus()[0]?.model ?? 'unknown',
    note: '基线只对同机有效；换机先 pnpm bench:baseline 重建',
  },
  created: new Date().toISOString().slice(0, 16),
  unit: 'ns',
  numbers,
};
const out = join(process.cwd(), 'docs', 'benchmarks', 'baseline.json');
mkdirSync(join(process.cwd(), 'docs', 'benchmarks'), { recursive: true });
writeFileSync(out, JSON.stringify(baseline, null, 2) + '\n');
console.log(`基线已写 ${out}（${Object.keys(numbers).length} 项，中位数 ns）`);
for (const [id, ns] of Object.entries(numbers)) console.log(`  ${id}  ${ns}`);
