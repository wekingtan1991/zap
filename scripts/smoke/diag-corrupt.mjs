// 独立小诊断：杀→破坏库→重启→dump 证据（S7 失败排查用，~15s）。
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, readFileSync, statSync, openSync, writeSync, closeSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const base = process.env.CARGO_TARGET_DIR || join(process.cwd(), 'src-tauri', 'target');
const exe = join(base, 'release', 'zap.exe');
const smokeData = join(process.env.APPDATA, 'com.ais.zap.smoke');
const logPath = join(smokeData, 'zap.log');
const dbPath = join(smokeData, 'zap.db');
let appPid = null;

const killSmokeZaps = () => spawnSync('powershell', ['-NoProfile', '-Command',
  `Get-Process zap -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq '${exe}' } | Stop-Process -Force`], { stdio: 'ignore' });
const alive = () => { try { process.kill(appPid, 0); return true; } catch { return false; } };

killSmokeZaps();
await sleep(500);

const p = spawn(exe, [], { env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9224' }, stdio: 'ignore' });
appPid = p.pid;
await sleep(5000);
console.log('预热实例存活:', alive());

spawnSync('taskkill', ['/PID', String(appPid), '/T', '/F'], { stdio: 'ignore' });
await sleep(800);
for (const s of ['-wal', '-shm']) { try { unlinkSync(dbPath + s); console.log(`unlink ${s}: ok`); } catch (e) { console.log(`unlink ${s}: ${e.code}`); } }

const fd = openSync(dbPath, 'r+');
writeSync(fd, Buffer.alloc(100, 0x00), 0, 100, 0);
closeSync(fd);
const head = readFileSync(dbPath).subarray(0, 16);
console.log('破坏后库头 hex:', head.toString('hex'), 'size:', statSync(dbPath).size);

spawn(exe, [], { env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9224' }, stdio: 'ignore' });
await sleep(6000);
console.log('重启后存活:', alive());
const log = readFileSync(logPath, 'utf8');
console.log('日志含 隔离重建:', log.includes('隔离重建'));
console.log('日志含 [E]:', log.includes('[E]'));
console.log('--- db\t 行 ---');
console.log(log.split('\n').filter((l) => l.includes('db\t')).slice(-4).join('\n'));
console.log('--- smokeData 文件清单 ---');
import { readdirSync } from 'node:fs';
for (const f of readdirSync(smokeData)) console.log('  ', f);
console.log('--- 重启后库头 ---');
console.log(readFileSync(dbPath).subarray(0, 16).toString('hex'), statSync(dbPath).size);
console.log('--- 日志尾 8 行 ---');
console.log(log.split('\n').slice(-8).join('\n'));
killSmokeZaps();
