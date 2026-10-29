/// 账号库备份提醒三态（纯函数，供设置页渲染与测试）：
/// never = 从未备份（数据只有库里一份，警示最强）；stale = 超过 30 天未导出；
/// ok = 其余。days 为距上次备份的天数（向下取整，时钟回拨时按 0 处理）
export type BackupReminder = {
  level: "never" | "ok" | "stale";
  days: number | null;
};

export const THIRTY_DAYS_MS = 30 * 86_400_000;

export function backupReminder(lastExportMs: number, nowMs: number): BackupReminder {
  if (!lastExportMs || lastExportMs <= 0) {
    return { level: "never", days: null };
  }
  const elapsed = nowMs - lastExportMs;
  const days = Math.max(0, Math.floor(elapsed / 86_400_000));
  return elapsed > THIRTY_DAYS_MS ? { level: "stale", days } : { level: "ok", days };
}
