// smoke 版构建（scripts/bench 持续方案，2026-11-07）。
//
// ⚠️ 为什么不走 `pnpm tauri build --no-bundle --config <smoke>`（实测踩坑）：
// 该链路产出的二进制是 dev 环境——页面导航 devUrl（http://localhost:1420）、
// 空白无 UI。根因在 tauri-2.12.3 src/lib.rs:32/318：dev/prod 由 tauri crate
// 的 `custom-protocol` feature 决定，这条 CLI 链路下该 feature 未启用。
// 而绕过 CLI 直接 cargo 编译又会丢 --config 的合并（identifier 隔离失效）。
// tauri-build lib.rs:71/682 支持 TAURI_CONFIG 环境变量做同款合并 →
// 本脚本走「CLI 同款合并 + cargo 直编带 feature」，两头都要。
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

const smokeConfig = readFileSync(join(process.cwd(), 'scripts', 'smoke', 'tauri.smoke.conf.json'), 'utf8');

console.log('[1/2] pnpm build（前端 dist）…');
const fe = spawnSync('pnpm', ['build'], { stdio: 'inherit', shell: true });
if (fe.status !== 0) process.exit(fe.status ?? 1);

console.log('[2/2] cargo build --release --features tauri/custom-protocol（TAURI_CONFIG 合并 smoke 配置）…');
const be = spawnSync('cargo', ['build', '--release', '--manifest-path', 'src-tauri/Cargo.toml', '--features', 'tauri/custom-protocol'], {
  stdio: 'inherit',
  env: { ...process.env, TAURI_CONFIG: smokeConfig },
});
if (be.status !== 0) process.exit(be.status ?? 1);
console.log('\nsmoke 版构建完成 ✅ identifier 已合并隔离；CDP 调试口由 e2e 运行时经 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS 注入');
