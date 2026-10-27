/// 后端 SearchItem 的线协议镜像（providers/mod.rs 的 camelCase 序列化）。
/// 放在插件目录：条目结构属于插件契约，注册表与各插件视图共用
export interface SearchItem {
  id: string;
  title: string;
  subtitle: string;
  kind: string;
  target: string;
  icon?: string | null;
  /// 置顶标记（结构化）：旧后端以「标题加 📌 前缀」表达，缺省时兜底解析
  pinned?: boolean;
  score: number;
}

/// 是否已置顶：优先读结构化字段；标题前缀是旧后端的 📌 约定，半新半旧
/// 会话兜底
export const isPinned = (it: SearchItem) => it.pinned ?? it.title.startsWith("📌");
