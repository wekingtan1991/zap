/// 后端 providers/ports.rs::PortEntry 的线协议镜像（camelCase 命名字段，不用位置元组）。
/// 新增字段需同步改 Rust 结构体的 serde 命名，避免位置解构那类静默错填。
export interface PortEntry {
  port: number;
  family: string;
  state: string;
  localAddr: string;
  pid: number;
  processName: string;
  processPath: string | null;
  /** 进程创建时间（Unix 毫秒），kill 前复核；权限不足时为 null */
  startedAt: number | null;
  /** 系统/自身/身份不明进程：前端禁用结束按钮，后端再挡一次 */
  protected: boolean;
}

/// 端口条目的 target 语义："all" = 全量列表，数字串 = 工作台初始过滤端口。
/// 纯函数，供 App 装配视图与单测锁定契约。
export function initialPortOf(target: string): number | null {
  if (target === "all" || target === "entry") return null;
  if (!/^\d{1,5}$/.test(target)) return null;
  const n = Number(target);
  return n > 0 && n <= 65535 ? n : null;
}
