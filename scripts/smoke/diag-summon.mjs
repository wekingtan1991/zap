// 专项诊断：呼出后 main target 的真实状态（scripts/bench 方案 2026-11-07）。
// 逐秒列 /json/list + 对 main 探 label/readystate/visibility，每次探测 3s 超时。
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const PORT = 9223;
const base = process.env.CARGO_TARGET_DIR || join(process.cwd(), 'src-tauri', 'target');
const exe = join(base, 'release', 'zap.exe');
if (!existsSync(exe)) { console.error('smoke exe 不存在'); process.exit(1); }

const pids = new Set();
const killAll = () => { for (const pid of pids) spawn('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' }); };
process.on('exit', killAll);

class Cdp {
  constructor(ws) { this.ws = ws; this.id = 0; this.pending = new Map(); }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('ws fail')); });
    const c = new Cdp(ws);
    ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && c.pending.has(m.id)) { const { res, rej } = c.pending.get(m.id); c.pending.delete(m.id); m.error ? rej(new Error(m.error.message)) : res(m.result); } };
    return c;
  }
  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((res, rej) => {
      const t = setTimeout(() => { this.pending.delete(id); rej(new Error('3s timeout')); }, 3000);
      this.pending.set(id, { res: (v) => { clearTimeout(t); res(v); }, rej: (e) => { clearTimeout(t); rej(e); } });
    });
  }
}

const app = spawn(exe, [], { env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}` }, stdio: 'ignore' });
pids.add(app.pid);
app.on('exit', (c, s) => console.log(`!! smoke 进程提前退出 code=${c} signal=${s}`));

const findMain = async () => {
  try {
    const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
    for (const page of list.filter((t) => t.type === 'page')) {
      const c = await Cdp.connect(page.webSocketDebuggerUrl);
      const probe = (await c.send('Runtime.evaluate', { expression: `(__TAURI_INTERNALS__?.metadata?.currentWindow?.label ?? '?') + '|' + document.readyState + '|' + document.visibilityState`, returnByValue: true })).result.value;
      c.ws.close();
      if (probe.startsWith('main|')) return { url: page.url, probe, page };
    }
  } catch { /* 本轮跳过（端点未就绪/会话失效均吞掉，靠外层重试） */ }
  return null;
};

// pre-summon
let main = null;
for (let i = 0; i < 40 && !main; i++) { await sleep(250); main = await findMain(); }
console.log(`pre-summon: ${main ? main.probe + ' @' + main.url : 'NOT FOUND'}`);

// 实验变量：呼出时是否持有一条打开的 DevTools 连接（e2e 与 diag 的唯一差异）
const held = main?.page ? await Cdp.connect(main.page.webSocketDebuggerUrl) : null;
console.log(`held connection: ${held ? 'OPEN（呼出期间保持）' : 'none'}`);

const second = spawn(exe, [], { stdio: 'ignore' });
pids.add(second.pid);

for (let i = 0; i < 12; i++) {
  await sleep(1000);
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json().catch(() => null);
  const pages = list ? list.filter((t) => t.type === 'page').map((t) => t.url.split('/').pop() || t.url) : ['LIST_FAIL'];
  const m = await findMain().catch((e) => `ERR:${e.message}`);
  console.log(`t+${i + 1}s pages=[${pages.join(', ')}] main=${m ? (m.probe ?? m) : 'none'}`);
}
killAll();
process.exit(0);
