/// 设置页脏态基线的序列化与线协议归一化（自 SettingsView 抽出的可测纯函数）。
/// 背景：基线写入与 dirty 对比曾是两处独立 JSON.stringify 字面量，键名
/// sc/scList 不一致导致加载完成后恒脏（Esc 必弹「放弃更改」确认、保存键
/// 常亮）——2026-11-01 真机实测发现。约定：任何基线写入点必须经
/// settingsSnapshot，dirty 对比亦然，杜绝两处形态漂移。
import type { AppSettings, Shortcut } from "./types";

/// 脏态对比快照：设置 + 捷径整体 JSON 化。字段改回原值仍算脏是
/// 可接受的保守语义，换取零侵入（不包 20+ 个 setSt 调用点）；
/// SettingsView 非热路径，stringify 代价可忽略。
export function settingsSnapshot(
  st: AppSettings,
  scList: Shortcut[]
): string {
  return JSON.stringify({ st, scList });
}

/// 后端返回归一化：补半新半旧会话缺失的新键（undefined 会导致保存时丢键）。
/// 挂载加载与保存后重拉共用，保证 st 与基线始终同一形态。
export function normalizeSettings(res: AppSettings): AppSettings {
  return {
    ...res,
    clipAutoPaste: res.clipAutoPaste ?? true,
    stageMax: res.stageMax ?? 50,
    stagedropEnabled: res.stagedropEnabled ?? true,
    stagedropOpenPanel: res.stagedropOpenPanel ?? false,
    stagedropPosition: res.stagedropPosition ?? "bottom_right",
    appearance: res.appearance ?? "dark",
    panelTransparency: res.panelTransparency ?? true,
    superpanelHotkey: res.superpanelHotkey ?? "",
    stageHotkey: res.stageHotkey ?? "",
    vaultHotkey: res.vaultHotkey ?? "",
    clipHotkey: res.clipHotkey ?? "",
    plugins: (res.plugins ?? []).map((p) => ({
      ...p,
      entries: p.entries ?? [],
      conflicts: p.conflicts ?? [],
    })),
  };
}
