/// 条目路由：runItem 的纯函数部分（Enter 与点击共用的分发决策）。
/// 返回"怎么处理"，执行仍由 AppView 装配——路由与副作用分离，可单测锁定契约
import { kindUi, type KindUi } from "./registry";
import type { SearchItem } from "./types";

export type ItemRoute =
  | { type: "form-add" }
  | { type: "import" }
  | { type: "activate"; ui: KindUi }
  | { type: "external" }
  | { type: "none" };

export function routeItem(it: SearchItem): ItemRoute {
  // 保护域伪条目：id 寻址（列表展示与普通条目无异，动作单独接）
  if (it.id === "vault:add") return { type: "form-add" };
  if (it.id === "vault:import") return { type: "import" };
  const ui = kindUi(it.kind);
  if (!ui) return { type: "external" }; // 未注册 kind：旧约定交后端
  if (ui.activate) return { type: "activate", ui };
  if (ui.external !== false) return { type: "external" };
  return { type: "none" };
}
