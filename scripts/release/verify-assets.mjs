#!/usr/bin/env node
/// Release 四件套自动化校验（checklist「更新链路」:31-32 的固化，2026-11-09）。
/// 背景：v0.19.0 发版时哈希一致性与 latest.json 内容未复验（对话实录
/// 2026-11-07 23:37 条），四件套下载走公开 API 无需 token。
///
/// 用法：node scripts/release/verify-assets.mjs [v0.19.0]
///   缺省 tag = v + package.json 的 version。产物落在
///   %TEMP%\zap-release-verify\<tag>\，供人工留档。
///
/// 校验项（任一失败 exit 1）：
///   1. GitHub Release 存在且四资产齐（setup.exe / .sig / latest.json / setup.sha256）
///   2. setup.sha256 声明值 == 实包 SHA-256 实算
///   3. latest.json：version 对 tag、url 指向本 tag 的 setup.exe、signature 与
///      .sig 资产内容一致
///   4. 签名 key id 与 tauri.conf.json 公钥 key id 配对（checklist :29 的
///      R16 结论：minisign 载荷不含公钥，配对只能比 key id）

import { createHash } from "node:crypto";
import { mkdtemp, readFile, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";

const PKG = await readJson("package.json");
const REPO = /github\.com[/:]([\w.-]+\/[\w.-]+?)(?:\.git)?$/.exec(
  PKG.repository?.url ?? ""
)?.[1];
if (!REPO) {
  console.error("✗ 无法从 package.json repository.url 解析 GitHub 仓库 slug");
  process.exit(1);
}
const TAG = process.argv[2] ?? `v${PKG.version}`;
const TMP = path.join(tmpdir(), "zap-release-verify", TAG);

let failures = 0;
const ok = (label, detail) => console.log(`  ✓ ${label}${detail ? ` — ${detail}` : ""}`);
const bad = (label, detail) => {
  failures++;
  console.log(`  ✗ ${label}${detail ? ` — ${detail}` : ""}`);
};

console.log(`zap release 校验 ${REPO} @ ${TAG}`);

// ---- 1. Release 元数据与四资产 ----
const release = await fetch(
  `https://api.github.com/repos/${REPO}/releases/tags/${TAG}`,
  { headers: { Accept: "application/vnd.github+json" } }
)
  .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`))))
  .catch((e) => {
    bad("GitHub API 读取 Release", `${e.message}（网络不通时先解决出口，本地校验可走 --file 模式另行扩展）`);
    finish();
  });

const assets = release.assets ?? [];
const pick = (suffix) => assets.find((a) => a.name === suffix || a.name.endsWith(suffix));
const exe = pick("-setup.exe");
const sig = pick("-setup.exe.sig");
const latest = pick("latest.json");
const sha = pick("setup.sha256");
for (const [label, asset] of [
  ["setup.exe 资产", exe],
  [".sig 资产", sig],
  ["latest.json 资产", latest],
  ["setup.sha256 资产", sha],
]) {
  asset ? ok(label, `${asset.name} ${asset.size}B`) : bad(label, "缺失");
}
if (!exe || !sig || !latest || !sha) finish();

// ---- 2. 下载四件并落盘留档 ----
await mkdir(TMP, { recursive: true });
const dl = async (asset, name) => {
  const r = await fetch(asset.browser_download_url);
  if (!r.ok) throw new Error(`${asset.browser_download_url} → HTTP ${r.status}`);
  const buf = Buffer.from(await r.arrayBuffer());
  await writeFile(path.join(TMP, name), buf);
  return buf;
};
let exeBuf, sigBuf, latestBuf, shaBuf;
try {
  [exeBuf, sigBuf, latestBuf, shaBuf] = await Promise.all([
    dl(exe, exe.name),
    dl(sig, sig.name),
    dl(latest, latest.name),
    dl(sha, sha.name),
  ]);
  ok("四资产下载", TMP);
} catch (e) {
  // 本机 github.com 下载域间歇不通（api 通、CDN 抖，2026-11-09 两跑两样）：
  // 回落上次成功运行留在 TMP 的产物。tag 资产按 tag 名寻址、发版不重发即不
  // 变；若怀疑留档过期，删 TMP 目录后重跑即可强制重下
  const local = (name) =>
    readFile(path.join(TMP, name)).catch(() => null);
  exeBuf = await local(exe.name);
  sigBuf = await local(sig.name);
  latestBuf = await local(latest.name);
  shaBuf = await local(sha.name);
  if (exeBuf && sigBuf && latestBuf && shaBuf) {
    ok("资产下载失败，回落本地留档", `${TMP}（删该目录重跑可强制重下）`);
  } else {
    bad(
      "资产下载",
      `${e.message}（且 ${TMP} 无完整留档；有代理时设 HTTPS_PROXY 并以 NODE_USE_ENV_PROXY=1 运行）`
    );
    finish();
  }
}

// ---- 3. setup.sha256 对实包哈希 ----
const declared = shaBuf.toString("utf8").trim().split(/\s+/)[0].toLowerCase();
const actual = createHash("sha256").update(exeBuf).digest("hex");
declared === actual
  ? ok("setup.sha256 == 实包 SHA-256", actual.slice(0, 16) + "…")
  : bad("setup.sha256 == 实包 SHA-256", `声明 ${declared.slice(0, 16)}… 实算 ${actual.slice(0, 16)}…`);

// ---- 4. latest.json 内容 ----
let meta;
try {
  meta = JSON.parse(latestBuf.toString("utf8"));
} catch {
  bad("latest.json 可解析");
  meta = {};
}
const ver = TAG.replace(/^v/, "");
meta.version === ver
  ? ok("latest.json version", meta.version)
  : bad("latest.json version", `期望 ${ver}，实为 ${meta.version}`);
const plat = meta.platforms?.["windows-x86_64"];
if (!plat) {
  bad("latest.json platforms.windows-x86_64", "缺失");
} else {
  (plat.url ?? "").includes(`/download/${TAG}/`) && plat.url.endsWith(exe.name)
    ? ok("latest.json url 指向本 tag 的 setup.exe", plat.url)
    : bad("latest.json url 指向本 tag 的 setup.exe", String(plat.url));
  (plat.signature ?? "").trim() === sigBuf.toString("utf8").trim()
    ? ok("latest.json signature == .sig 资产")
    : bad("latest.json signature == .sig 资产", "两处签名内容不一致");
}

// ---- 5. 签名 key id 与公钥 key id 配对 ----
const conf = await readJson("src-tauri/tauri.conf.json");
const sigKeyid = keyidOfMinisign(sigBuf.toString("utf8"));
const pubKeyid = keyidOfMinisignPub(conf.plugins?.updater?.pubkey ?? "");
sigKeyid && pubKeyid && sigKeyid === pubKeyid
  ? ok("签名 key id == 公钥 key id", sigKeyid)
  : bad("签名 key id == 公钥 key id", `签名 ${sigKeyid ?? "?"} 公钥 ${pubKeyid ?? "?"}（R16：错配=全绿发版但用户更新静默失败）`);

finish();

// ---- 工具 ----
async function readJson(file) {
  return JSON.parse(await readFile(file, "utf8"));
}

/** 载荷（alg(2) + keyid(8) + …）中的 key id → 展示 hex（与 pubkey 注释行/
 * checklist :29 记录同序：原始字节序需反序——实测 pub 载荷原始序为
 * 152764D17E8957C9，注释展示 C957897ED1642715，互为反序；签名与公钥同一
 * 序列化约定，两侧同变换比较即配对） */
function keyidOfPayload(b64) {
  const raw = Buffer.from(b64, "base64");
  if (raw.length < 10) return null;
  return Buffer.from(raw.subarray(2, 10)).reverse().toString("hex").toUpperCase();
}

/** 签名文件 → key id hex。发布资产的 .sig 与 latest.json 的 signature 字段
 * 同构：minisign 签名文本（untrusted / 载荷行 / trusted / 全局签名行）整体
 * 再 base64 的单行嵌套——先剥一层再找载荷行；本地原生 .sig 已是文本则跳过 */
function keyidOfMinisign(raw) {
  let text = raw.trim();
  if (!text.includes("\n")) {
    try {
      const decoded = Buffer.from(text, "base64").toString("utf8");
      if (decoded.startsWith("untrusted comment")) text = decoded;
    } catch {
      /* 非 base64 或非签名文本，按原文解析 */
    }
  }
  // 载荷行 74B → 100 字符；下限取 50 容 pub 行（42B→56 字符）。内层首条
  // ≥50 的行即签名载荷行（注释行含空格不命中，全局签名行在其后）
  const b64 = text.split(/\r?\n/).find((l) => /^[A-Za-z0-9+/]{50,}={0,2}$/.test(l.trim()));
  return b64 ? keyidOfPayload(b64) : null;
}

/** 公钥（tauri.conf.json 存的是 minisign pub 文件整体再 base64）→ key id hex */
function keyidOfMinisignPub(pubkeyField) {
  let text = pubkeyField;
  try {
    text = Buffer.from(pubkeyField, "base64").toString("utf8");
  } catch {
    /* 字段若直接是文件内容/路径，按原文解析 */
  }
  const b64 = text.split(/\r?\n/).find((l) => /^[A-Za-z0-9+/]{50,}={0,2}$/.test(l.trim()));
  return b64 ? keyidOfPayload(b64) : null;
}

function finish() {
  console.log(
    failures === 0 ? "\n全部通过 ✓" : `\n${failures} 项未过 ✗（详情见上，留档目录 ${TMP}）`
  );
  process.exit(failures === 0 ? 0 : 1);
}
