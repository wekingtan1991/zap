/// 插件前端注册表：kind → 徽标文案/配色 与 激活行为。
/// 后端 Plugin 产出的条目按 kind 在这里声明 UI；新增插件 = 后端实现 Plugin trait
/// + 在这里加一行注册，App.tsx 的分发与渲染不再随插件增长。
import type { ReactNode } from "react";
import type { BadgeVariant } from "@/components/ui/badge";
import { VaultRowPin } from "./vault/VaultRowPin";
import { isPinned, type SearchItem } from "./types";

/// 激活上下文：AppView 提供的动作入口，各 kind 的 activate 按需取用
export interface ItemActions {
  openVaultDetail: (it: SearchItem) => void;
  /// 打开 JSON 工作台（按条目 target 定内容源：q=搜索词 / entry=空待粘贴 / 行号回查）
  openJson: (it: SearchItem) => void;
  /// 打开文本工具工作台（target 语义同 openJson）
  openTool: (it: SearchItem) => void;
  runCommand: (target: string) => void;
  /// 默认动作：交给后端 open_item（启动应用 / 打开文件 / 激活窗口等）
  openExternal: (it: SearchItem) => void;
  /// vault 条目的行尾置顶
  vaultPin: (it: SearchItem) => void;
  /// 剪贴板条目的行尾固定
  clipPin: (it: SearchItem) => void;
}

export interface KindUi {
  /// 徽标文案
  label: string;
  /// shadcn Badge variant（badge.tsx 的语义色）
  badge: BadgeVariant;
  /// Enter/点击的专属动作；缺省时若 external 不为 false 则落到 openExternal
  activate?: (it: SearchItem, act: ItemActions) => void;
  /// 纯展示条目（info 等）关闭外发动作
  external?: boolean;
  /// 行尾动作区（如 vault 的置顶按钮）
  rowAction?: (it: SearchItem, act: ItemActions) => ReactNode;
}

const REGISTRY: Record<string, KindUi> = {
  app: { label: "应用", badge: "app" },
  clip: {
    label: "剪贴板",
    badge: "clip",
    rowAction: (it, act) => (
      <VaultRowPin pinned={isPinned(it)} onPin={() => act.clipPin(it)} />
    ),
  },
  file: { label: "文件", badge: "file" },
  win: { label: "窗口", badge: "win" },
  recent: { label: "最近", badge: "recent" },
  settings: { label: "设置", badge: "settings" },
  vault: {
    label: "账号",
    badge: "vault",
    activate: (it, act) => act.openVaultDetail(it),
    rowAction: (it, act) => (
      <VaultRowPin pinned={isPinned(it)} onPin={() => act.vaultPin(it)} />
    ),
  },
  web: { label: "搜索", badge: "web" },
  url: { label: "网址", badge: "url" },
  json: {
    label: "JSON",
    badge: "json",
    activate: (it, act) => act.openJson(it),
  },
  tool: {
    label: "工具",
    badge: "tool",
    activate: (it, act) => act.openTool(it),
  },
  cmd: {
    label: "功能",
    badge: "cmd",
    activate: (it, act) => act.runCommand(it.target),
  },
  info: { label: "状态", badge: "info", external: false },
};

/// 未注册 kind 返回 undefined：渲染退回原文案 + secondary 徽标，激活落到 openExternal
export const kindUi = (kind: string): KindUi | undefined => REGISTRY[kind];
