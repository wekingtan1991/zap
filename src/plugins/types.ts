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

/// 文本剪贴板条目的行号：寻址键在 id（clip:{rowid}），target 是内容本体——
/// 按 target 取数是 NaN（合并粘贴首版踩过：target 取数 → NaN → 卫兵静默吞）。
/// 仅对 kind="clip" 有意义，其余条目返回 NaN
export const clipRowId = (it: SearchItem): number =>
  it.kind === "clip" ? Number(it.id.replace(/^clip:/, "")) : Number.NaN;

/// 图片剪贴板条目的行号：与文本 clip 相反，行号在 target（回贴/预览按 target
/// 寻址），id 是 clipimg:{rowid} 标识。两种 kind 寻址键不同是线协议事实，
/// 各用各的助手取，别互相套
export const clipImageRowId = (it: SearchItem): number =>
  it.kind === "clipimg" ? Number(it.target) : Number.NaN;
