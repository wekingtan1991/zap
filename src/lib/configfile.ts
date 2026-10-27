/// 配置文件（导出/导入）的线协议与导入合并逻辑。
/// 后端镜像：src-tauri/src/providers/appconfig.rs 的 ConfigFile
/// （camelCase 序列化，settings 各项 None 落为 null = 文件未携带此键）

/// 搜索捷径（后端 shortcuts::Shortcut 的线协议镜像，与 SettingsView 内定义一致）
export interface ConfigShortcut {
  keyword: string;
  name: string;
  url: string;
}

/// 配置文件整体。settings 缺键 = 该项保留当前值（前向兼容新版本文件）；
/// plugins 只含 id + enabled，标题/入口词由本机插件 meta 下发
export interface ConfigFilePayload {
  kind: string;
  version: number;
  exportedAt: string | null;
  appVersion: string | null;
  settings: Partial<{
    hotkey: string | null;
    vaultHotkey: string | null;
    clipHotkey: string | null;
    superpanelHotkey: string | null;
    stageHotkey: string | null;
    clipEnabled: boolean | null;
    clipMax: number | null;
    clipAutoPaste: boolean | null;
    stageMax: number | null;
    stagedropEnabled: boolean | null;
    pwdClearSecs: number | null;
    autostart: boolean | null;
    everythingAutostart: boolean | null;
    appearance: string | null;
    panelTransparency: boolean | null;
  }>;
  plugins: { id: string; enabled: boolean }[];
  shortcuts: ConfigShortcut[];
}

/// 设置表单快照（SettingsView 的 AppSettings 结构约束；结构化兼容即可）
export interface SettingsSnapshot {
  hotkey: string;
  vaultHotkey: string;
  clipHotkey: string;
  superpanelHotkey: string;
  stageHotkey: string;
  clipEnabled: boolean;
  clipMax: number;
  clipAutoPaste: boolean;
  stageMax: number;
  stagedropEnabled: boolean;
  pwdClearSecs: number;
  autostart: boolean;
  everythingAutostart: boolean;
  appearance: string;
  panelTransparency: boolean;
  plugins: { id: string; enabled: boolean }[];
}

/// 设置页闭集枚举（与 Select 选项一致）：文件值不在枚举内按缺项处理，
/// 避免回填出选择器里不存在的值
export const CLIP_MAX_CHOICES = [100, 500, 1000, 5000];
export const STAGE_MAX_CHOICES = [30, 50, 100, 200];
export const PWD_CLEAR_CHOICES = [0, 30, 45, 60];
const APPEARANCE_CHOICES = ["dark", "light", "system"];

/// 导入文件 → 设置表单合并：文件缺项/越界值保留当前值；插件开关只取现存 id
/// （文件可能来自新/旧版本，插件清单以本机为准）；捷径整单替换（校验在保存时做）
export function mergeImportedConfig<T extends SettingsSnapshot>(
  current: T,
  file: ConfigFilePayload
): T {
  const c = file.settings;
  const hotkey = (v: string | null | undefined, fb: string) =>
    typeof v === "string" ? v : fb;
  const bool = (v: boolean | null | undefined, fb: boolean) =>
    typeof v === "boolean" ? v : fb;
  const num = (v: number | null | undefined, choices: number[], fb: number) =>
    v !== null && v !== undefined && choices.includes(v) ? v : fb;
  const appearance =
    typeof c?.appearance === "string" &&
    APPEARANCE_CHOICES.includes(c.appearance)
      ? c.appearance
      : current.appearance;
  return {
    ...current,
    hotkey: hotkey(c?.hotkey, current.hotkey),
    vaultHotkey: hotkey(c?.vaultHotkey, current.vaultHotkey),
    clipHotkey: hotkey(c?.clipHotkey, current.clipHotkey),
    superpanelHotkey: hotkey(c?.superpanelHotkey, current.superpanelHotkey),
    stageHotkey: hotkey(c?.stageHotkey, current.stageHotkey),
    clipEnabled: bool(c?.clipEnabled, current.clipEnabled),
    clipMax: num(c?.clipMax, CLIP_MAX_CHOICES, current.clipMax),
    clipAutoPaste: bool(c?.clipAutoPaste, current.clipAutoPaste),
    stageMax: num(c?.stageMax, STAGE_MAX_CHOICES, current.stageMax),
    stagedropEnabled: bool(c?.stagedropEnabled, current.stagedropEnabled),
    pwdClearSecs: num(c?.pwdClearSecs, PWD_CLEAR_CHOICES, current.pwdClearSecs),
    autostart: bool(c?.autostart, current.autostart),
    everythingAutostart: bool(c?.everythingAutostart, current.everythingAutostart),
    appearance,
    panelTransparency: bool(c?.panelTransparency, current.panelTransparency),
    plugins: current.plugins.map((p) => {
      const hit = (file.plugins ?? []).find((x) => x.id === p.id);
      return hit ? { ...p, enabled: !!hit.enabled } : p;
    }),
  };
}

/// 文件里实际携带的设置项个数（导入 toast 用）
export function countImportedSettings(file: ConfigFilePayload): number {
  return Object.values(file.settings ?? {}).filter(
    (v) => v !== null && v !== undefined
  ).length;
}
