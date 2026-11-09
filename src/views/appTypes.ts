import type { SearchItem } from "@/plugins/types";
import type { VaultDetail } from "@/plugins/vault/VaultDetailView";
import type { AccountForm } from "@/plugins/vault/VaultFormView";
import type { RevealInfo } from "@/plugins/vault/VaultRevealOverlay";

/// 工作台内容源：入口进入 = 空文本等待粘贴；输入识别（万能输入框）= 携带
/// 进入时刻的搜索词全文；剪贴板识别 = 按行号回查呼出时刻的内容
export type WorkbenchSource =
  | { kind: "text"; text: string }
  | { kind: "clip"; rowid: number }
  | { kind: "empty" };

/// 面板视图状态机：同一时刻恰有一个子页面，取代原先 8 个并列的布尔/对象状态。
/// 新增子页面 = 加一个 kind + 渲染分支，zap://shown 复位与互斥天然成立。
/// vault 域动作 hook（plugins/vault/useVaultActions）按此类型操作子视图
export type View =
  | { kind: "main" }
  | { kind: "settings" }
  | { kind: "stage" }
  | { kind: "detail"; data: VaultDetail; askDelete?: boolean }
  | { kind: "form"; data: AccountForm }
  | { kind: "reveal"; data: RevealInfo }
  | { kind: "import" }
  | { kind: "json"; source: WorkbenchSource; backQuery: string }
  | { kind: "tool"; source: WorkbenchSource; backQuery: string }
  | { kind: "port"; port: number | null; backQuery: string }
  | { kind: "clipimg"; rowid: number; backQuery: string };

export const VAULT_ADD_ENTRY: SearchItem = {
  id: "vault:add",
  title: "＋ 新增账号",
  subtitle: "标题 / 用户名 / 密码 / 网址 / 备注",
  kind: "info",
  target: "",
  score: -1,
};

export const VAULT_IMPORT_ENTRY: SearchItem = {
  id: "vault:import",
  title: "智能导入（粘贴文本批量识别）",
  subtitle: "从记事本 / 聊天记录粘贴账号文本，自动解析字段后校对入库",
  kind: "info",
  target: "",
  score: -2,
};
