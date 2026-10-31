// 热键呼出冒烟（自主测试工具，2026-10-31 评审实施批萊档）：
// 1. 记 zap.log 基线行数
// 2. PowerShell keybd_event 注入呼出热键（默认 ctrl+alt+space，本机存档若
//    改过用 --hotkey ctrl+space 形式传入，如 `node scripts/hotkey-smoke.mjs ctrl+space`）
// 3. 对比日志新增：出现 `shown\temit` = 呼出链路（热键回调→toggle_panel→show）走通
// 前置：zap.exe 已在运行（startup 行已在日志中，可先核对 hotkey= 值）。
// 注入用临时 ps1 落 %TEMP%（纯 ASCII，无 GBK 风险），用完即删。
import { readFileSync, writeFileSync, unlinkSync } from "node:fs";
import { execSync } from "node:child_process";

const LOG = `${process.env.APPDATA}\\com.ais.zap\\zap.log`;
const hotkey = process.argv[2] ?? "ctrl+alt+space";

// 解析组合键为虚拟键序列（ctrl=0x11, alt=0x12, shift=0x10, space=0x20, 字母/数字=其 ASCII）
const VK = { ctrl: 0x11, alt: 0x12, shift: 0x10, space: 0x20 };
const seq = hotkey
  .split("+")
  .map((k) => VK[k] ?? (k.length === 1 ? k.toUpperCase().charCodeAt(0) : null));
if (seq.some((v) => v === null)) {
  console.error(`无法解析热键: ${hotkey}`);
  process.exit(1);
}

const before = readFileSync(LOG, "utf8").split("\n").filter(Boolean).length;
console.log("基线日志行数:", before);

const PS1 = `${process.env.TEMP}\\zap-hotkey-inject.ps1`;
const body = [
  'Add-Type -TypeDefinition @"',
  "using System;",
  "using System.Runtime.InteropServices;",
  "public static class KB {",
  '  [DllImport("user32.dll")]',
  "  public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);",
  "}",
  '"@',
  ...seq.flatMap((vk) => [
    `[KB]::keybd_event(0x${vk.toString(16)},0,0,[UIntPtr]::Zero)`,
    "Start-Sleep -Milliseconds 60",
  ]),
  ...seq
    .slice()
    .reverse()
    .flatMap((vk) => [
      `[KB]::keybd_event(0x${vk.toString(16)},0,2,[UIntPtr]::Zero)`,
      "Start-Sleep -Milliseconds 60",
    ]),
  'Write-Output "injected"',
].join("\n");
writeFileSync(PS1, body);

try {
  execSync(`powershell -NoProfile -ExecutionPolicy Bypass -File "${PS1}"`, {
    stdio: "inherit",
    timeout: 30000,
  });
} finally {
  unlinkSync(PS1);
}

await new Promise((r) => setTimeout(r, 1500));
const lines = readFileSync(LOG, "utf8").split("\n").filter(Boolean);
console.log("注入后日志行数:", lines.length);
lines.slice(before).forEach((l) => console.log("NEW:", l));
const shown = lines.slice(before).some((l) => l.includes("shown\temit"));
console.log(
  shown
    ? "SMOKE-OK：呼出链路走通（热键回调→toggle_panel→show）"
    : "SMOKE-FAIL：未检测到呼出（核对运行中实例的 hotkey= 是否与本脚本组合键一致，见 startup 行）"
);
process.exit(shown ? 0 : 1);
