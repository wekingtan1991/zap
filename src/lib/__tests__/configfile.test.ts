/// mergeImportedConfig / countImportedSettings：导入回填合并契约
import { describe, expect, it } from "vitest";
import {
  countImportedSettings,
  mergeImportedConfig,
  type ConfigFilePayload,
  type SettingsSnapshot,
} from "../configfile";

const current: SettingsSnapshot = {
  hotkey: "ctrl+alt+space",
  vaultHotkey: "",
  clipHotkey: "",
  superpanelHotkey: "alt+s",
  stageHotkey: "",
  clipEnabled: true,
  clipMax: 500,
  clipAutoPaste: true,
  stageMax: 50,
  stagedropEnabled: true,
  stagedropOpenPanel: false,
  pwdClearSecs: 45,
  autostart: false,
  everythingAutostart: true,
  appearance: "dark",
  panelTransparency: true,
  plugins: [
    { id: "json", enabled: true },
    { id: "texttools", enabled: true },
  ],
};

const file = (settings: ConfigFilePayload["settings"]): ConfigFilePayload => ({
  kind: "zap-config",
  version: 1,
  exportedAt: "2026-10-27 12:00:00",
  appVersion: "0.11.0",
  settings,
  plugins: [{ id: "texttools", enabled: false }],
  shortcuts: [{ keyword: "gh", name: "GitHub", url: "https://github.com/search?q={query}" }],
});

describe("mergeImportedConfig", () => {
  it("全量文件逐项回填", () => {
    const out = mergeImportedConfig(current, file({
      hotkey: "ctrl+shift+space",
      vaultHotkey: "alt+v",
      clipHotkey: "alt+c",
      superpanelHotkey: "alt+f",
      stageHotkey: "alt+t",
      clipEnabled: false,
      clipMax: 1000,
      clipAutoPaste: false,
      stageMax: 200,
      stagedropEnabled: false,
      stagedropOpenPanel: true,
      pwdClearSecs: 30,
      autostart: true,
      everythingAutostart: false,
      appearance: "light",
      panelTransparency: false,
    }));
    expect(out.hotkey).toBe("ctrl+shift+space");
    expect(out.vaultHotkey).toBe("alt+v");
    expect(out.clipHotkey).toBe("alt+c");
    expect(out.superpanelHotkey).toBe("alt+f");
    expect(out.stageHotkey).toBe("alt+t");
    expect(out.clipEnabled).toBe(false);
    expect(out.clipMax).toBe(1000);
    expect(out.clipAutoPaste).toBe(false);
    expect(out.stageMax).toBe(200);
    expect(out.stagedropEnabled).toBe(false);
    expect(out.stagedropOpenPanel).toBe(true);
    expect(out.pwdClearSecs).toBe(30);
    expect(out.autostart).toBe(true);
    expect(out.everythingAutostart).toBe(false);
    expect(out.appearance).toBe("light");
    expect(out.panelTransparency).toBe(false);
  });

  it("缺项保留当前值（null 与 settings 缺字段同义）", () => {
    const out = mergeImportedConfig(current, file({
      hotkey: null,
      pwdClearSecs: null,
      appearance: null,
    }));
    expect(out.hotkey).toBe("ctrl+alt+space");
    expect(out.pwdClearSecs).toBe(45);
    expect(out.appearance).toBe("dark");
    // 未提及的键全部保持
    expect(out.clipMax).toBe(500);
    expect(out.autostart).toBe(false);
  });

  it("越界值按缺项处理（闭集枚举不回填非法值）", () => {
    const out = mergeImportedConfig(current, file({
      clipMax: 12345 as number,
      stageMax: 7 as number,
      pwdClearSecs: 99 as number,
      appearance: "sepia" as string,
    }));
    expect(out.clipMax).toBe(500);
    expect(out.stageMax).toBe(50);
    expect(out.pwdClearSecs).toBe(45);
    expect(out.appearance).toBe("dark");
  });

  it("插件开关按 id 合并，本机没有的插件忽略", () => {
    const out = mergeImportedConfig(current, file({}));
    expect(out.plugins.find((p) => p.id === "texttools")?.enabled).toBe(false);
    expect(out.plugins.find((p) => p.id === "json")?.enabled).toBe(true);
  });

  it("current 对象不被就地修改", () => {
    mergeImportedConfig(current, file({ clipEnabled: false }));
    expect(current.clipEnabled).toBe(true);
    expect(current.plugins.find((p) => p.id === "texttools")?.enabled).toBe(true);
  });
});

describe("countImportedSettings", () => {
  it("只统计文件实际携带的键", () => {
    expect(countImportedSettings(file({ hotkey: "a", clipMax: 500 }))).toBe(2);
    expect(countImportedSettings(file({}))).toBe(0);
  });
});
