#!/usr/bin/env node
/**
 * 敏感信息门禁（check:secrets）
 *
 *   node scripts/check-secrets.mjs            扫描 git 已跟踪的全部文本文件
 *   node scripts/check-secrets.mjs <file...>  只扫指定文件（自检 / 定向排查）
 *
 * 规则只描述业界通用凭证特征，不针对任何具体历史值；
 * 常用范例（RFC 5737 文档网段、example.com、测试号段等）走白名单。
 * 命中非白名单即以退出码 1 结束，且日志只输出截断预览，避免二次泄露。
 */
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const SELF = 'scripts/check-secrets.mjs';
const MAX_BYTES = 2_000_000;

const R = (id, re) => ({ id, re });
const RULES = [
  R('PRIVATE_KEY', /-----BEGIN [A-Z ]*PRIVATE KEY-----|PuTTY-User-Key-File/gi),
  R('CLOUD_TOKEN', /(?:AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,}|sk-[A-Za-z0-9_-]{20,}/g),
  R('VCS_TOKEN', /(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|npm_[A-Za-z0-9]{30,})/g),
  R('CHAT_TOKEN', /xox[baprs]-[A-Za-z0-9-]{10,}/g),
  R('JWT', /eyJ[A-Za-z0-9_-]{8,}[.]eyJ[A-Za-z0-9_-]{8,}[.][A-Za-z0-9_-]{8,}/g),
  R('CONN_STRING', /(?:postgres|mysql|mongodb|redis|amqp|mssql|jdbc):[/][/][^ "'<]{6,}/gi),
  R('WEBHOOK', /https:[/][/](?:hooks[.]slack[.]com|discord[.]com[/]api[/]webhooks|oapi[.]dingtalk[.]com|qyapi[.]weixin[.]qq[.]com)[/]/g),
  // 数字类规则加双向边界：避免把版本号 1.4.1.1009、时间戳、测试向量当成 IP / 手机号
  R('PRIVATE_IP', /(?<![0-9.])(?:10[.][0-9]{1,3}[.][0-9]{1,3}[.][0-9]{1,3}|172[.](?:1[6-9]|2[0-9]|3[01])[.][0-9]{1,3}[.][0-9]{1,3}|192[.]168[.][0-9]{1,3}[.][0-9]{1,3})(?::[0-9]{2,5})?(?![0-9.])/g),
  R('IPV4', /(?<![0-9.])[0-9]{1,3}[.][0-9]{1,3}[.][0-9]{1,3}[.][0-9]{1,3}(?::[0-9]{2,5})?(?![0-9.])/g),
  R('PHONE_CN', /(?<![0-9])1[3-9][0-9]{9}(?![0-9])/g),
  R('EMAIL', /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+[.][A-Za-z]{2,}/g),
  R('SECRET_ASSIGN', /(?:password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|credential)[ ]*[:=][ ]*["'][^"' ]{6,}["']/gi),
  R('OS_USER_PATH', /[A-Za-z]:.Users.[A-Za-z0-9._-]{2,}/g),
];

const LOCK_FILE = /(?:^|[/])(?:Cargo[.]lock|pnpm-lock[.]yaml|package-lock[.]json|yarn[.]lock)$/;
const TEST_PHONES = new Set([
  '13800138000', '13800138001', '13900138000', '13900138001',
  '13900000000', '13900000001',
]);
const ALLOW_IP = /^(?:127[.]0[.]0[.]1|0[.]0[.]0[.]0|255[.]255[.]255[.]255|192[.]0[.]2[.][0-9]{1,3}|198[.]51[.]100[.][0-9]{1,3}|203[.]0[.]113[.][0-9]{1,3})(?::[0-9]{2,5})?$/;

function allowed(id, v, file) {
  if (LOCK_FILE.test(file) && (id === 'PHONE_CN' || id === 'IPV4')) return true;
  if (id === 'IPV4' && ALLOW_IP.test(v)) return true;
  if (id === 'PHONE_CN' && TEST_PHONES.has(v)) return true;
  if (id === 'EMAIL') {
    if (/@(?:example[.](?:com|org|net)|users[.]noreply[.]github[.]com)$/i.test(v)) return true;
    if (/@[0-9]x[.]png$/i.test(v)) return true; // 图标文件名，如 AppIcon-20x20@2x.png
  }
  if (id === 'SECRET_ASSIGN' && /(?:example|placeholder|redacted|dummy|sample|password123|••|<)/i.test(v)) return true;
  if (id === 'OS_USER_PATH' && /(?:张三|<|example|placeholder)/.test(v)) return true;
  return false;
}

// 只输出前 4 字符，门禁日志本身不能成为第二次泄露
const preview = (v) => (v.length <= 8 ? '[redacted]' : v.slice(0, 4) + '…(' + v.length + ' chars)');

let files;
const args = process.argv.slice(2);
if (args.length) {
  files = args;
} else {
  try {
    files = execFileSync('git', ['ls-files'], { encoding: 'utf8' }).trim().split('\n').filter(Boolean);
  } catch {
    console.error('[check:secrets] 无法读取 git 已跟踪文件列表，请在仓库根目录运行');
    process.exit(1);
  }
}

const findings = [];
let scanned = 0;
for (const file of files) {
  if (file === SELF) continue; // 规则自身的字面量会命中自己
  let buf;
  try {
    buf = readFileSync(file);
  } catch {
    continue;
  }
  if (buf.length > MAX_BYTES || buf.includes(0)) continue;
  scanned += 1;
  const lines = buf.toString('utf8').split('\n');
  for (let i = 0; i < lines.length; i += 1) {
    for (const rule of RULES) {
      rule.re.lastIndex = 0;
      let m;
      while ((m = rule.re.exec(lines[i])) !== null) {
        const v = m[0];
        if (allowed(rule.id, v, file)) continue;
        findings.push(`${file}:${i + 1} [${rule.id}] ${preview(v)}`);
      }
    }
  }
}

if (findings.length) {
  console.error(`[check:secrets] 发现 ${findings.length} 处疑似敏感信息（已扫描 ${scanned} 个文件）：`);
  for (const f of findings.slice(0, 30)) console.error('  ' + f);
  if (findings.length > 30) console.error(`  … 另有 ${findings.length - 30} 处`);
  console.error('[check:secrets] FAIL：确认是误报请补进白名单，确认是凭证请先移除并轮换。');
  process.exit(1);
}
console.log(`[check:secrets] 已扫描 ${scanned} 个文件，无敏感信息命中`);
