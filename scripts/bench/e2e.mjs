// 端到端性能实测（scripts/bench 持续方案，2026-11-07）。
// 跑在 smoke 实例（scripts/smoke/tauri.smoke.conf.json 构建，identifier 隔离）上，
// 经 CDP 附加 WebView2 注入测量钩子（运行时注入，零源码插桩）：
//   M1 冷启动 = 进程起点→CDP 端点就绪（→首帧 paint entry）
//   M3 搜索   = 键入→invoke→回执→DOM commit 四段（真实 Input 管线派发字符）
// ⚠️ 实测纪律（diag-summon 两轮实验的结论）：WebView2 调试服务器生命周期不可
// 长程依赖——diag 模式「每操作全新短会话 + 快速重试」全程稳定，长持会话在
// 呼出后曾整体失联两次。故所有页面操作都走 withMain（新会话用完即关），
// 钩子/打点活在页面 JS 状态里，与会话无关。
// 前提：pnpm build:smoke（见 build-smoke.mjs 头注释——CLI --no-bundle 链路产
// dev 二进制的坑，不能直接用 pnpm tauri build --no-bundle --config）
// 用法：pnpm bench:e2e（本机端口 9223，测量后进程必被清理）
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const PORT = 9223;
const base = process.env.CARGO_TARGET_DIR || join(process.cwd(), 'src-tauri', 'target');
const exe = join(base, 'release', 'zap.exe');
if (!existsSync(exe)) {
  console.error(`smoke exe 不存在：${exe}\n先跑 pnpm build:smoke`);
  process.exit(1);
}

const pids = new Set();
const killAll = () => {
  for (const pid of pids) spawn('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
};
process.on('exit', killAll);
process.on('SIGINT', () => { killAll(); process.exit(130); });

// ---- CDP 最小客户端（Node 26 原生 WebSocket） ----
class Cdp {
  constructor(ws) { this.ws = ws; this.id = 0; this.pending = new Map(); }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('ws 连接失败')); });
    const c = new Cdp(ws);
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && c.pending.has(msg.id)) { const { res, rej } = c.pending.get(msg.id); c.pending.delete(msg.id); msg.error ? rej(new Error(msg.error.message)) : res(msg.result); }
    };
    return c;
  }
  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((res, rej) => {
      const timer = setTimeout(() => { this.pending.delete(id); rej(new Error(`CDP ${method} 8s 超时`)); }, 8000);
      this.pending.set(id, {
        res: (v) => { clearTimeout(timer); res(v); },
        rej: (e) => { clearTimeout(timer); rej(e); },
      });
    });
  }
}

const PROBE = `window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label + '|' + document.readyState`;

const jsonList = async () => (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();

// 每操作全新会话（diag 实证模式）：定位 main → 用完即关，失败重试。
// 钩子与打点在页面 JS 状态里，跨会话存续。
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

// 逐字符真实键入（CDP Input 管线，含 React onChange）。
// ⚠️ 实证校验：keyDown+text 在 WebView2 下若未落进输入框（React onChange 未触发），
// 退回 Input.insertText（IME 级插入，直达 DOM value 变更）
const typeText = async (text) => {
  for (const ch of text) {
    await withMain((c) => c.send('Input.dispatchKeyEvent', {
      type: 'keyDown', key: ch, text: ch, unmodifiedText: ch,
      windowsVirtualKeyCode: ch.toUpperCase().charCodeAt(0), code: 'Key' + ch.toUpperCase(),
    }));
    await withMain((c) => c.send('Input.dispatchKeyEvent', { type: 'keyUp', key: ch, windowsVirtualKeyCode: ch.toUpperCase().charCodeAt(0), code: 'Key' + ch.toUpperCase() }));
    await sleep(15);
  }
  const value = await withMain((c) => c.send('Runtime.evaluate', { expression: `document.querySelector('input')?.value ?? ''`, returnByValue: true })).then((r) => r.result.value);
  if (!value.includes(text)) {
    console.log(`  键入未落框（value="${value}"）→ 退回 Input.insertText`);
    await withMain((c) => c.send('Input.insertText', { text }));
  }
  // 末键后同会话直读页面时钟（typeEnd 误差 ~几 ms，供「末键→渲染」计算）
  return await withMain((c) => c.send('Runtime.evaluate', { expression: 'performance.now()', returnByValue: true })).then((r) => r.result.value);
};

const snapshot = async () =>
  JSON.parse((await withMain((c) => c.send('Runtime.evaluate', { expression: `JSON.stringify(window.__zapBench)`, returnByValue: true }))).result.value);

const HOOKS = `(() => {
  if (window.__zapBench) return 'already:true';
  const rec = { invokes: [], lastCommit: 0, commits: [] };
  // 层1：__TAURI_INTERNALS__.invoke 补丁——实测前端在模块加载期捕获引用，
  // 运行时补丁被绕开（rows 增长而 invokes=0），留作兜底
  const it = window.__TAURI_INTERNALS__;
  if (it?.invoke) {
    const orig = it.invoke.bind(it);
    it.invoke = (cmd, args, opts) => {
      const slot = { cmd, t0: performance.now(), layer: 'invoke' };
      rec.invokes.push(slot);
      return orig(cmd, args, opts).finally(() => { slot.t1 = performance.now(); });
    };
  }
  // 层2（主钩子）：Tauri v2 IPC 走 fetch('ipc://…')，fetch 属性调用时解析，
  // 运行时补丁可靠命中；cmd 取 URL 末段
  if (window.fetch) {
    const origFetch = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const url = String(input?.url ?? input ?? '');
      if (!url.includes('ipc://')) return origFetch(input, init);
      const slot = { cmd: decodeURIComponent(url.split('/').pop() || 'ipc'), t0: performance.now(), layer: 'fetch' };
      rec.invokes.push(slot);
      return origFetch(input, init).finally(() => { slot.t1 = performance.now(); });
    };
  }
  // 层3：WebView2 宿主对象 IPC（chrome.webview.postMessage）——invoke 的真实
  // 传输层。宿主方法可能不可写，try 兜住；只有发送侧无回执侧（t1 缺省）
  try {
    const cw = window.chrome?.webview;
    if (cw?.postMessage && !cw.__zapPatched) {
      const orig = cw.postMessage.bind(cw);
      cw.postMessage = (...args) => {
        let cmd = 'postMessage';
        try {
          const a = args[0];
          cmd = (typeof a === 'string' ? JSON.parse(a)?.cmd : a?.cmd) ?? 'postMessage';
        } catch { /* cmd 解析失败不拦消息 */ }
        rec.invokes.push({ cmd, t0: performance.now(), layer: 'cw' });
        return orig(...args);
      };
      try { cw.__zapPatched = true; } catch { /* 只读则忽略 */ }
    }
  } catch { /* 宿主对象不可补丁 */ }
  new MutationObserver(() => {
    const t = performance.now();
    rec.lastCommit = t;
    // 首渲染时点要靠 commit 序列取（lastCommit 会被图标等异步后到 mutations 顶走）
    rec.commits.push(t);
    if (rec.commits.length > 300) rec.commits.shift();
  }).observe(document.body, { childList: true, subtree: true });
  window.__zapBench = rec;
  return 'installed:true';
})()`;

// ---- 主流程 ----
const t0 = Date.now();
console.log(`启动 smoke 实例：${exe}（CDP :${PORT}）`);
const app = spawn(exe, [], { env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}` }, stdio: 'ignore' });
pids.add(app.pid);

const cdpReadyAt = await withMain(async () => Date.now(), 120); // 端点+页面就绪即计时
const paintMs = await withMain((c) => c.send('Runtime.evaluate', {
  expression: `(() => { const p = performance.getEntriesByType('paint')[0]; return p ? Math.round(p.startTime) : -1; })()`,
})).then((r) => r.result.value);
console.log(`M1 进程起点→CDP 就绪: ${cdpReadyAt - t0}ms · 文档起点→首帧: ${paintMs}ms`);

// 呼出面板：单实例二次启动信号（lib.rs:63），零代码改动
const second = spawn(exe, [], { stdio: 'ignore' });
pids.add(second.pid);
await sleep(600);

let focused = false;
for (let i = 0; i < 20 && !focused; i++) {
  focused = await withMain(async (c) => {
    const v = (await c.send('Runtime.evaluate', { expression: `document.activeElement?.tagName || ''`, returnByValue: true })).result.value;
    if (v !== 'INPUT') await c.send('Runtime.evaluate', { expression: `document.querySelector('input')?.focus()` });
    return v === 'INPUT';
  }).catch(() => false);
  if (!focused) await sleep(200);
}
if (!focused) {
  const dump = await withMain((c) => c.send('Runtime.evaluate', {
    expression: `JSON.stringify({inputs: document.querySelectorAll('input').length, active: document.activeElement?.tagName, vis: document.visibilityState, hasFocus: document.hasFocus(), rows: document.querySelectorAll('.zap-item').length})`,
    returnByValue: true,
  })).then((r) => r.result.value).catch((e) => `dump失败:${e.message}`);
  throw new Error(`呼出后输入框未获得焦点——面板可能未显示。现场=${dump}`);
}
console.log('面板已呼出，输入框有焦点 ✅');
const paintAfter = await withMain((c) => c.send('Runtime.evaluate', {
  expression: `(() => { const p = performance.getEntriesByType('paint')[0]; return p ? Math.round(p.startTime) : -1; })()`,
})).then((r) => r.result.value).catch(() => -1);
if (paintAfter >= 0 && paintMs < 0) console.log(`M1 首帧（呼出后实读）: ${paintAfter}ms`);

// 测量钩子：重载注入——钩子必须在应用模块加载前注册（防 invoke 引用被
// 模块加载期捕获绕开补丁）。addScriptToEvaluateOnNewDocument 于文档起点
// 自动执行，__zapBench 直接存在于页面 JS 状态（跨 CDP 会话存续）
const hookBoot = await withMain(async (c) => {
  await c.send('Page.enable');
  await c.send('Page.addScriptToEvaluateOnNewDocument', { source: HOOKS });
  await c.send('Page.reload', {});
  for (let i = 0; i < 50; i++) {
    await sleep(200);
    const s = (await c.send('Runtime.evaluate', {
      expression: `document.readyState + '|' + (!!window.__zapBench)`,
      returnByValue: true,
    })).result.value;
    if (s === 'complete|true') return s;
  }
  return 'timeout';
});
let hooksReady = hookBoot === 'complete|true';
if (hooksReady) {
  console.log('重载注入完成（钩子于文档起点生效）✅');
} else {
  console.log(`⚠️ 重载注入未生效（${hookBoot}）→ 退回运行时安装，invoke 分段可能缺失`);
}

// 重载后重新聚焦输入框
let refocused = false;
for (let i = 0; i < 20 && !refocused; i++) {
  refocused = await withMain(async (c) => {
    const v = (await c.send('Runtime.evaluate', { expression: `document.activeElement?.tagName || ''`, returnByValue: true })).result.value;
    if (v !== 'INPUT') await c.send('Runtime.evaluate', { expression: `document.querySelector('input')?.focus()` });
    return v === 'INPUT';
  }).catch(() => false);
  if (!refocused) await sleep(200);
}
if (!refocused) throw new Error('重载后输入框未获得焦点');
if (!hooksReady) {
  const install = await withMain((c) => c.send('Runtime.evaluate', { expression: HOOKS, returnByValue: true })).then((r) => r.result.value);
  if (install !== 'installed:true') throw new Error(`钩子安装失败: ${install}`);
  console.log('钩子运行时安装 ✅');
}

// React onChange 兜底：原生 value setter + input 事件（重载键入未触发时用）
const SET_VALUE = (q) => `(() => {
  const el = document.querySelector('input');
  if (!el) return 'no-input';
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
  setter.call(el, ${JSON.stringify(q)});
  el.dispatchEvent(new Event('input', { bubbles: true }));
  return el.value;
})()`;

// 等收敛：lastCommit 300ms 不再变化，返回快照与 search invoke（相对 pre 新增）
const waitForSearch = async (pre, deadlineMs = 6000) => {
  let last = -1, stable = 0, snap = pre;
  const deadline = Date.now() + deadlineMs;
  while (Date.now() < deadline && stable < 3) {
    await sleep(150);
    snap = await snapshot();
    stable = snap.lastCommit === last ? stable + 1 : 0;
    last = snap.lastCommit;
  }
  const searchInvoke = snap.invokes.filter((x) => x.cmd === 'search' && !pre.invokes.some((y) => y.t0 === x.t0)).pop();
  return { snap, searchInvoke };
};

// M3：三组查询，每组 键入 →（未触发则 setter 兜底）等收敛 → 收四段时间
const QUERIES = ['set', 'json', 'note'];
const results = [];
for (const q of QUERIES) {
  const pre = await snapshot();
  const typeStart = await withMain((c) => c.send('Runtime.evaluate', { expression: 'performance.now()', returnByValue: true })).then((r) => r.result.value);
  const typeEnd = await typeText(q);
  let { snap, searchInvoke } = await waitForSearch(pre);
  if (!searchInvoke) {
    // invoke 钩子未命中（IPC 引用模块期捕获/宿主对象不可写）→ setter 兜底确保值就位
    await withMain((c) => c.send('Runtime.evaluate', { expression: SET_VALUE(q), returnByValue: true }));
    ({ snap, searchInvoke } = await waitForSearch(pre, 4000));
  }
  const typeMs = Math.round(typeEnd - typeStart);
  // 主指标：末键之后的首次 DOM commit（结果渲染），不含图标等后到 mutations
  const firstRender = snap.commits?.find((c) => c > typeEnd) ?? null;
  const finalMs = firstRender !== null ? Math.round(firstRender - typeEnd) : null;
  if (!searchInvoke) {
    results.push({ query: q, typeMs, finalMs, totalOnly: true });
    continue;
  }
  const hasReply = typeof searchInvoke.t1 === 'number';
  results.push({
    query: q,
    layer: searchInvoke.layer,
    typeMs,
    typeToInvoke: Math.round(searchInvoke.t0 - typeStart),
    invokeToReply: hasReply ? Math.round(searchInvoke.t1 - searchInvoke.t0) : null,
    replyToCommit: hasReply && firstRender !== null ? Math.round(firstRender - searchInvoke.t1) : null,
    sendToCommit: !hasReply && firstRender !== null ? Math.round(firstRender - searchInvoke.t0) : null,
    finalMs,
  });
  // 清空（原生 setter，键入 BackSpace 在 WebView2 下不可靠）再下一组
  await withMain((c) => c.send('Runtime.evaluate', { expression: SET_VALUE(''), returnByValue: true }));
  await sleep(400);
}

console.log('\nM3 搜索端到端（smoke 实例，主指标=末键→渲染；键入节奏 15ms/字符）：');
for (const r of results) {
  if (r.miss) console.log(`${r.query.padEnd(8)} 未捕获（超时）· 现场=${r.dbg}`);
  else if (r.totalOnly) console.log(`${r.query.padEnd(8)} 键入全程 ${r.typeMs}ms · 末键→渲染 ${r.finalMs ?? '?'}ms（invoke 钩子未命中，分段不可用）`);
  else if (r.invokeToReply !== null) console.log(`${r.query.padEnd(8)} 键入全程 ${r.typeMs}ms · 键入→invoke ${r.typeToInvoke} · invoke→回执 ${r.invokeToReply} · 回执→渲染 ${r.replyToCommit} · 末键→渲染 ${r.finalMs}ms [${r.layer}]`);
  else console.log(`${r.query.padEnd(8)} 键入全程 ${r.typeMs}ms · 键入→IPC发送 ${r.typeToInvoke} · 发送→渲染 ${r.sendToCommit} · 末键→渲染 ${r.finalMs}ms [${r.layer}]`);
}

killAll();
console.log('\n测量完成，进程已清理 ✅');
process.exit(0);
