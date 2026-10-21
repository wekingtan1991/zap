/// 后端 SearchItem 的线协议镜像（providers/mod.rs 的 camelCase 序列化）。
/// 放在插件目录：条目结构属于插件契约，注册表与各插件视图共用
export interface SearchItem {
  id: string;
  title: string;
  subtitle: string;
  kind: string;
  target: string;
  icon?: string | null;
  score: number;
}

/// 是否已置顶：后端给置顶条目的标题加了 📌 前缀（列表与详情共用该约定）
export const isPinned = (it: SearchItem) => it.title.startsWith("📌");
