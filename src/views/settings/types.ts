/// 设置页线协议类型（自 SettingsView 拆出，审计 E3）：设置快照、插件状态、
/// 搜索捷径——分区组件与主干共用。后端镜像：providers/appconfig.rs 的
/// AppSettingsDto / PluginInfo / shortcuts::Shortcut（camelCase 线协议）

export interface PluginState {
  id: string;
  title: string;
  hint: string;
  enabled: boolean;
  /// 搜索入口词（后端 PluginMeta.entries 声明化下发）
  entries: string[];
  /// 入口词冲突（与其他插件或搜索捷径关键词重合，后端现算下发）
  conflicts: string[];
}

export interface AppSettings {
  hotkey: string;
  /// 账号库直达热键（空串 = 未绑定）
  vaultHotkey: string;
  /// 快速粘贴直达热键（空串 = 未绑定）
  clipHotkey: string;
  /// 超级面板直达热键（空串 = 未绑定）
  superpanelHotkey: string;
  /// 中转站直达热键（空串 = 未绑定）
  stageHotkey: string;
  clipEnabled: boolean;
  clipMax: number;
  /// 剪贴板条目选中后自动粘贴到原窗口
  clipAutoPaste: boolean;
  /// 中转站未固定条目容量上限
  stageMax: number;
  /// 全局拖拽落区条开关
  stagedropEnabled: boolean;
  /// 拖入落区条松手后自动呼出中转站面板
  stagedropOpenPanel: boolean;
  /// 落区条停靠角（bottom_right/bottom_left/top_right/top_left）
  stagedropPosition: string;
  pwdClearSecs: number;
  autostart: boolean;
  everythingAutostart: boolean;
  /// 外观模式（dark/light/system）
  appearance: string;
  /// 透明面板（亚克力窗口特效）
  panelTransparency: boolean;
  /// 可开关插件清单（后端 meta 下发，设置页「插件」分区渲染）
  plugins: PluginState[];
}

/// 搜索捷径（后端 shortcuts::Shortcut 的线协议镜像）：
/// keyword 触发词 / name 展示名 / url 含 {query} 占位的模板
export interface Shortcut {
  keyword: string;
  name: string;
  url: string;
}
