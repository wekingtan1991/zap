/// 脏态基线序列化与设置归一化的契约测试。背景：基线写入与 dirty 对比曾是
/// 两处独立 JSON.stringify 字面量，键名 sc/scList 不一致导致设置页加载完成后
/// 恒脏（Esc 必弹「放弃更改」、保存键常亮）——2026-11-01 真机实测发现。
/// 回归口径：快照形态唯一来源在 settingsSnapshot，归一化必须幂等。
import { describe, expect, it } from "vitest";
import { normalizeSettings, settingsSnapshot } from "../dirty";
import type { AppSettings, Shortcut } from "../types";

const settings: AppSettings = {
  hotkey: "ctrl+space",
  vaultHotkey: "alt+v",
  clipHotkey: "",
  superpanelHotkey: "alt+s",
  stageHotkey: "",
  clipEnabled: true,
  clipMax: 500,
  clipAutoPaste: true,
  stageMax: 50,
  stagedropEnabled: true,
  stagedropOpenPanel: false,
  stagedropPosition: "bottom_right",
  pwdClearSecs: 45,
  autostart: false,
  everythingAutostart: false,
  appearance: "dark",
  panelTransparency: true,
  plugins: [
    {
      id: "json",
      title: "JSON 工具",
      hint: "",
      enabled: true,
      entries: ["json"],
      conflicts: [],
    },
  ],
};

const shortcuts: Shortcut[] = [
  { keyword: "gh", name: "GitHub", url: "https://github.com/{query}" },
];

describe("settingsSnapshot", () => {
  it("同内容快照相等（基线与 dirty 对比共用同一键形态——键名漂移回归锁）", () => {
    const a = settingsSnapshot(settings, shortcuts);
    const b = settingsSnapshot({ ...settings }, [...shortcuts]);
    expect(a).toBe(b);
  });

  it("设置或捷径任一变化快照即不同", () => {
    const base = settingsSnapshot(settings, shortcuts);
    expect(
      settingsSnapshot({ ...settings, clipMax: 1000 }, shortcuts)
    ).not.toBe(base);
    expect(settingsSnapshot(settings, [])).not.toBe(base);
  });
});

describe("normalizeSettings", () => {
  it("补齐缺失的后续新增键（半新会话容错，含插件数组缺段）", () => {
    const raw = { ...settings } as Record<string, unknown>;
    delete raw.clipAutoPaste;
    delete raw.stagedropPosition;
    delete raw.plugins;
    const fixed = normalizeSettings(raw as unknown as AppSettings);
    expect(fixed.clipAutoPaste).toBe(true);
    expect(fixed.stagedropPosition).toBe("bottom_right");
    expect(fixed.plugins).toEqual([]);
  });

  it("幂等：归一化结果再归一化不变（加载→保存重拉→再归一化不漂移）", () => {
    const once = normalizeSettings(settings);
    expect(normalizeSettings(once)).toEqual(once);
  });
});
