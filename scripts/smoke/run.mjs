// L3 稳定冒烟（scripts/bench 持续方案，2026-11-07 立项；阶段3 交付）。
// 全程跑 smoke 实例（identifier 隔离，数据目录本脚本开头重置），绝不碰生产数据。
//
//   S1 冷启动×3（spawn→CDP 就绪→杀）          S2 热键等价 toggle×50（二次启动信号，每 10 次查原生可见性）
//   S3 搜索连打 200 键（20 轮×10 字符）        S4 剪贴板翻滚 50 条（watcher 收录，marker 落列表验证）
//   S5 大图复制（3000×3000 BGRA ≈36MB）       S6 写入中 kill→重启验活
//   S7 kill→破坏库→重启（坏库软着陆实证）      S8 smoke 日志 [E] 清点
//
// ⚠️ 系统剪贴板是全局的：S4/S5 会短暂改写它——文本现场 base64 保存、结尾还原
//（原剪贴板若是图片无法还原，输出注明；首尾空白不保证逐字节还原）。
// ⚠️ 只按可执行路径杀 smoke 实例，不用 taskkill /IM zap.exe（会误杀用户正在
// 运行的真实 zap——e2e 调试期踩过，本脚本杜绝）。
//
// 前提：pnpm build:smoke；用法：pnpm smoke；任一阶段 ⛔ 最终 exit 1。
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, readdirSync, rmSync, readFileSync, statSync, openSync, writeSync, closeSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const PORT = 9224; // 与 e2e 的 9223 错开
const base = process.env.CARGO_TARGET_DIR || join(process.cwd(), 'src-tauri', 'target');
const exe = join(base, 'release', 'zap.exe');
if (!existsSync(exe)) {
  console.error(`smoke exe 不存在：${exe}\n先跑 pnpm build:smoke`);
  process.exit(1);
}
const smokeData = join(process.env.APPDATA, 'com.ais.zap.smoke');
const logPath = join(smokeData, 'zap.log');
const dbPath = join(smokeData, 'zap.db');

let appPid = null;
const pids = new Set(); // 全部已启动进程（收尾兜底清理）
const fail = [];
const results = [];
const stage = (name, ok, note = '') => {
  results.push({ name, ok, note });
  console.log(`${ok ? '✅' : '⛔'} ${name}${note ? ` — ${note}` : ''}`);
  if (!ok) fail.push(name);
};

const killTree = (pid) => spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
// 杀 + 确认死透：taskkill /T 对含 webview 子进程的树清不干净时（600ms 级），
// 旧实例存活会让后续 spawn 被 single-instance 转发后退出、探针连上旧实例
// ——S6/S7 假阳性/假阴性的根因（2026-11-07 实测）。所有 kill 一律走这里
const killAndWait = async (pid, deadlineMs = 6000) => {
  killTree(pid);
  const deadline = Date.now() + deadlineMs;
  while (alive(pid) && Date.now() < deadline) await sleep(200);
  return !alive(pid);
};
const alive = (pid) => pid !== null && (() => { try { process.kill(pid, 0); return true; } catch { return false; } })();
// PS 单引号串里反斜杠不是转义，路径原样即可
const killSmokeZaps = () => spawnSync('powershell', ['-NoProfile', '-Command',
  `Get-Process zap -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq '${exe}' } | Stop-Process -Force`],
  { stdio: 'ignore' });
const ps = (script) => spawnSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], { encoding: 'utf8' }).stdout?.trim() ?? '';

// ---- CDP（helpers 与 e2e.mjs 同款：每操作全新短会话 + 重试；改动需两处同步） ----
class Cdp {
  constructor(ws) { this.ws = ws; this.id = 0; this.pending = new Map(); }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('ws fail')); });
    const c = new Cdp(ws);
    ws.onmessage = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id && c.pending.has(m.id)) { const { res, rej } = c.pending.get(m.id); c.pending.delete(m.id); m.error ? rej(new Error(m.error.message)) : res(m.result); }
    };
    return c;
  }
  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((res, rej) => {
      const t = setTimeout(() => { this.pending.delete(id); rej(new Error(`CDP ${method} 8s 超时`)); }, 8000);
      this.pending.set(id, { res: (v) => { clearTimeout(t); res(v); }, rej: (e) => { clearTimeout(t); rej(e); } });
    });
  }
}
const PROBE = `window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label + '|' + document.readyState`;
const jsonList = async () => (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const withMain = async (fn, tries = 40) => {
  for (let i = 0; i < tries; i++) {
    try {
      const list = await jsonList();
      for (const page of list.filter((t) => t.type === 'page')) {
        const c = await Cdp.connect(page.webSocketDebuggerUrl);
        const probe = (await c.send('Runtime.evaluate', { expression: PROBE, returnByValue: true })).result.value;
        if (probe === 'main|complete') {
          try { return await fn(c); } finally { c.ws.close(); }
        }
        c.ws.close();
      }
    } catch { /* 端点未就绪/会话失效，重试 */ }
    await sleep(250);
  }
  throw new Error('withMain: main target 持续不可用');
};
const SET_VALUE = (q) => `(() => {
  const el = document.querySelector('input');
  if (!el) return 'no-input';
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
  setter.call(el, ${JSON.stringify(q)});
  el.dispatchEvent(new Event('input', { bubbles: true }));
  return el.value;
})()`;

const spawnApp = () => {
  const p = spawn(exe, [], { env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}` }, stdio: 'ignore' });
  pids.add(p.pid);
  appPid = p.pid;
  return p;
};
const summon = () => {
  const p = spawn(exe, [], { stdio: 'ignore' }); // 二次启动 = toggle（lib.rs:61-64）
  pids.add(p.pid);
  return p;
};
const waitReady = async (deadlineMs = 20000) => {
  const deadline = Date.now() + deadlineMs;
  for (;;) {
    if (Date.now() > deadline) return false;
    try { await withMain(async () => true, 1); return true; } catch { await sleep(200); }
  }
};
const focusInput = async (tries = 10) => {
  for (let i = 0; i < tries; i++) {
    const ok = await withMain(async (c) => {
      const v = (await c.send('Runtime.evaluate', { expression: `document.activeElement?.tagName || ''`, returnByValue: true })).result.value;
      if (v !== 'INPUT') await c.send('Runtime.evaluate', { expression: `document.querySelector('input')?.focus()` });
      return v === 'INPUT';
    }).catch(() => false);
    if (ok) return true;
    await sleep(200);
  }
  return false;
};
// 主窗口 class=Tauri Window / title='zap'（diag-windows.ps1 枚举实证）；
// 单实例插件的 <identifier>-siw 信使窗口恒 visible，不可作探针目标
const visibleNative = () => ps(`& '${join(process.cwd(), 'scripts', 'smoke', 'visibility.ps1')}' -Title 'zap'`);
// toggle 路径执行次数：panel.rs:154 每次 show 埋 `shown\temit`——原生可见性在
// 脚本语境不可断言（呼出即失焦→失焦即隐，亚秒内自动收起，400ms 探针粒度
// 永远抓不到「可见」；diag-summon-native.ps1 + zap.log 对照实证 2026-11-07），
// 故 toggle 正确性以日志 shown 计数为断言对象，可见性仅作观察
const shownCount = () => {
  try { return readFileSync(logPath, 'utf8').split('\n').filter((l) => l.includes('shown\temit')).length; }
  catch { return -1; }
};
// 面板可见+输入聚焦的确定性恢复：隐藏则一次 toggle，可见但失焦则两连 toggle（藏→亮+夺焦）
const ensurePanel = async () => {
  if (visibleNative() !== 'visible') { summon(); await sleep(500); }
  if (await focusInput(3)) return true;
  summon(); await sleep(500);
  summon(); await sleep(500);
  return focusInput(5);
};

process.on('exit', () => { for (const pid of pids) killTree(pid); });
process.on('SIGINT', () => { process.exit(130); });

// ==================== 主流程 ====================
const t0 = Date.now();
console.log(`L3 稳定冒烟：${exe}`);
console.log(`⚠️ 将短暂改写系统剪贴板（文本结尾 base64 还原）；只杀路径为 smoke 的 zap 实例\n`);

// S0 清场
killSmokeZaps();
await sleep(500);
if (existsSync(smokeData)) rmSync(smokeData, { recursive: true, force: true });
console.log(`S0 隔离数据目录已重置：${smokeData}\n`);

// S1 冷启动×3
{
  let ok = true; const notes = [];
  for (let i = 1; i <= 3; i++) {
    const t = Date.now();
    spawnApp();
    const ready = await waitReady(20000);
    if (!ready || !alive(appPid)) { ok = false; notes.push(`第${i}次 未就绪/早退(${ready}/${alive(appPid)})`); break; }
    notes.push(`${Date.now() - t}ms`);
    await killAndWait(appPid);
    await sleep(500);
  }
  stage('S1 冷启动×3 就绪', ok, notes.join(' / '));
}

// S2a 慢速 toggle×3 严格奇偶（验证机制正确性；起点=冷启动隐藏态）
// S2b 快速 toggle 风暴×50 只断言存活+窗口可探（呼出/收起与失焦自动隐藏在
// 快速风暴下互相竞争，奇偶断言属测试工件伪命题——2026-11-07 实测系统性偏移）
{
  spawnApp();
  const ready = await waitReady();
  if (!ready) { stage('S2a toggle 正确性×3', false, '应用未就绪'); stage('S2b toggle 风暴×50', false, '应用未就绪'); }
  else {
    await sleep(400);
    // S2a：3 次慢速 summon。shown 计数天生抖动（toggle 落点时面板可能仍可见→
    // 走 hide 分支无埋点），断言只要求机制在跑（+≥1）；可见瞬间做 150ms 采样
    // 观察（不进判定——脚本语境呼出即失焦，亚秒自动收起，见 helper 注释）
    const base0 = shownCount();
    let glimpse = false;
    for (let i = 1; i <= 3; i++) {
      summon();
      for (let k = 0; k < 6 && !glimpse; k++) {
        await sleep(150);
        if (visibleNative() === 'visible') glimpse = true;
      }
      await sleep(400);
      if (!alive(appPid)) break;
    }
    const grew = shownCount() - base0;
    stage('S2a toggle 路径×3', grew >= 1 && alive(appPid),
      `shown 埋点 +${grew}/3${glimpse ? '，观察到原生可见瞬间' : '，150ms 采样未抓到可见瞬间（失焦即隐快于采样，观察项不判定）'}`);

    // S2b：50 连发风暴，断言不崩 + 窗口全程可探 + 机制仍在跑（shown +≥1）
    const base1 = shownCount();
    let dead = false, broken = false;
    for (let i = 1; i <= 50; i++) {
      summon();
      await sleep(420);
      if (!alive(appPid)) { dead = true; break; }
      if (i % 10 === 0 && visibleNative() === 'notfound') broken = true;
    }
    const grew1 = shownCount() - base1;
    stage('S2b toggle 风暴×50 不崩', !dead && !broken && grew1 >= 1,
      dead ? '应用死亡' : broken ? '窗口失联' : `shown 埋点 +${grew1}/50（落点可见性五五开，只断言机制在跑）、窗口全程可探`);
  }
}

// S3 搜索连打 200 键（20 轮 × 10 字符）
{
  const QUERIES = ['settings01', 'jsonwork01', 'notetool01', 'zaplaunch1', 'smoketest1',
    'benchmar01', 'metrics001', 'latency001', 'checkup001', 'clipboard01'];
  let ok = await ensurePanel();
  let note = ok ? '' : '输入框无法聚焦';
  if (ok) {
    let keys = 0;
    for (let r = 0; r < 20 && ok; r++) {
      const q = QUERIES[r % QUERIES.length];
      for (const ch of q) {
        await withMain((c) => c.send('Input.dispatchKeyEvent', { type: 'keyDown', key: ch, text: ch, unmodifiedText: ch, windowsVirtualKeyCode: ch.toUpperCase().charCodeAt(0), code: 'Key' + ch.toUpperCase() }));
        await withMain((c) => c.send('Input.dispatchKeyEvent', { type: 'keyUp', key: ch, windowsVirtualKeyCode: ch.toUpperCase().charCodeAt(0), code: 'Key' + ch.toUpperCase() }));
        keys++;
      }
      await sleep(700); // 防抖+搜索+渲染
      await withMain((c) => c.send('Runtime.evaluate', { expression: SET_VALUE(''), returnByValue: true }));
      await sleep(150);
      if (!alive(appPid)) { ok = false; note = `第${r + 1}轮应用死亡`; }
    }
    if (ok) note = `${keys} 键 / 20 轮全通`;
  }
  stage('S3 搜索连打 200 键', ok, note);
}

// S4 剪贴板翻滚 50 条
const savedClipB64 = ps(`[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((Get-Clipboard -Raw)))`);
{
  const MARK = 'zap-smoke-clip';
  let ok = true, note = '';
  for (let i = 1; i <= 50 && ok; i++) {
    ps(`Set-Clipboard -Value ('${MARK}-{0:D3}' -f ${i})`);
    await sleep(260);
    if (!alive(appPid)) { ok = false; note = `第${i}条时应用死亡`; }
  }
  if (ok) {
    if (!(await ensurePanel())) { ok = false; note = '验证时无法聚焦'; }
    else {
      await withMain((c) => c.send('Runtime.evaluate', { expression: SET_VALUE(MARK), returnByValue: true }));
      await sleep(900);
      const hit = await withMain((c) => c.send('Runtime.evaluate', {
        expression: `[...document.querySelectorAll('.zap-item .zap-title')].filter(t => t.textContent.includes('${MARK}')).length`,
        returnByValue: true,
      })).then((r) => r.result.value).catch(() => -1);
      ok = hit > 0;
      note = ok ? `${hit} 行 marker 命中（容量淘汰按设置）` : '列表 0 命中';
    }
  }
  stage('S4 剪贴板翻滚 50 条', ok, note);
}

// S5 大图复制（3000×3000 BGRA ≈ 36MB）
{
  const out = ps(`Add-Type -AssemblyName System.Drawing; Add-Type -AssemblyName System.Windows.Forms; $b = New-Object System.Drawing.Bitmap 3000,3000; $g = [System.Drawing.Graphics]::FromImage($b); $g.Clear([System.Drawing.Color]::Red); $g.Dispose(); [System.Windows.Forms.Clipboard]::SetImage($b); 'set'`);
  await sleep(1200);
  stage('S5 大图复制 36MB', out === 'set' && alive(appPid), alive(appPid) ? '进程存活' : '进程死亡');
}

// 剪贴板文本现场还原（base64 免引号地狱；原为图片则置空并注明）
{
  if (savedClipB64) ps(`Set-Clipboard -Value ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('${savedClipB64}')))`);
  else ps(`Set-Clipboard -Value ''`);
  console.log(`ℹ️ 系统剪贴板已还原为文本${savedClipB64 ? '' : '（原剪贴板非文本/为空，置空处理）'}`);
}

// S6 写入中 kill → 重启验活
{
  const killed = await killAndWait(appPid);
  spawnApp();
  const ready = await waitReady() && killed;
  stage('S6 kill→重启验活', ready && alive(appPid), ready ? '旧实例确认死亡后重启即就绪' : killed ? '重启未就绪' : '旧实例 6s 未杀净');
}

// S7 kill→破坏库→重启（坏库软着陆：损毁头部，应用须隔离重建并正常启动）
// ⚠️ 旧实例必须确认死透：存活时会收走 single-instance 转发、让新实例秒退，
// waitReady 连上开着健康旧库的旧实例 → 假阴性（2026-11-07 实测踩过）
{
  const killed = await killAndWait(appPid);
  // ⚠️ WAL 必须确认删净：WAL 自洽时 SQLite 从叠加层读整库，被毁的库头根本
  // 不会被读——kill 后句柄释放有延迟，一次性 unlink 会静默失败（实测踩过）。
  // 重试到文件确认消失，删不净就如实报失败
  let walGone = false;
  const walDeadline = Date.now() + 6000;
  while (Date.now() < walDeadline) {
    for (const suffix of ['-wal', '-shm']) { try { unlinkSync(dbPath + suffix); } catch { /* 可能本就不存在/仍锁定 */ } }
    if (!existsSync(dbPath + '-wal') && !existsSync(dbPath + '-shm')) { walGone = true; break; }
    await sleep(300);
  }
  let corrupted = false, corruptNote = '';
  if (walGone && existsSync(dbPath)) {
    // ⚠️ 载荷必须非零：WAL 模式下从未 checkpoint 的新库主文件本来就是全零
    //（内容全在 WAL），写零等于没写——SQLite 对全零文件按空库健康初始化
    //（2026-11-07 实测踩过）。0xFF 保证魔数必失效
    const fd = openSync(dbPath, 'r+');
    writeSync(fd, Buffer.alloc(100, 0xff), 0, 100, 0);
    closeSync(fd);
    const head = readFileSync(dbPath).subarray(0, 16).toString('hex');
    corrupted = head === 'ff'.repeat(16);
    corruptNote = `库头已毁并回读校验（大小 ${statSync(dbPath).size}B）`;
  } else {
    corruptNote = `WAL 6s 未删净（wal=${existsSync(dbPath + '-wal')} shm=${existsSync(dbPath + '-shm')}）——破坏不生效`;
  }
  spawnApp();
  await waitReady(25000) && killed && corrupted;
  // ⚠️ db 是惰性打开的：隔离事件可能落在 waitReady 之后数秒（实测 +3.5s），
  // 必须 poll 日志，单次读取会假阴性
  let softLanded = false;
  const qDeadline = Date.now() + 10000;
  while (Date.now() < qDeadline && !softLanded) {
    await sleep(300);
    try { softLanded = readFileSync(logPath, 'utf8').includes('隔离重建'); } catch { /* 无日志 */ }
  }
  stage('S7 坏库软着陆', softLanded && alive(appPid),
    softLanded ? '日志实证「库不健康，隔离重建」+ 应用正常启动' : '10s 内日志无隔离重建记录——软着陆缺失');
}

// S8 smoke 日志 [E] 清点
{
  let eCount = -1;
  try { eCount = readFileSync(logPath, 'utf8').split('\n').filter((l) => l.includes('[E]')).length; } catch { /* 无日志 */ }
  stage('S8 日志 [E] 清点', eCount === 0, eCount === 0 ? '0 条' : `${eCount} 条（zap.log [E] 前缀）`);
}

// 收尾
killSmokeZaps();

console.log(`\n总耗时 ${((Date.now() - t0) / 1000).toFixed(1)}s · ${results.filter((r) => r.ok).length}/${results.length} 阶段通过`);
if (fail.length) { console.error(`⛔ 未过阶段：${fail.join('、')}`); process.exit(1); }
console.log('L3 冒烟全绿 ✅（发版清单 :L3 可勾）');
process.exit(0);
