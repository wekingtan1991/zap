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

/// 剪贴板条目行号（文本 clip 与图片 clipimg 线协议统一）：target = 行号字符串，
/// 内容/图片本体只在动作时按行号回查（回贴/预览/仅复制），不随列表过 IPC。
/// id（clip:{rowid} / clipimg:{rowid}）仍是跨列表操作的寻址键。历史注记：
/// 合并粘贴首版文本 clip 的 target 曾是内容本体、行号只在 id——按 target 取数
/// 恒 NaN 静默吞掉勾选（当时以 id 提取修）；clipimg 一直是 target=行号，
/// 两 kind 同构后助手合一
export const clipRowId = (it: SearchItem): number =>
  it.kind === "clip" || it.kind === "clipimg" ? Number(it.target) : Number.NaN;
