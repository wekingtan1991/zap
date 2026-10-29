import { describe, expect, it } from "vitest";
import { THIRTY_DAYS_MS, backupReminder } from "../vaultBackup";

const DAY = 86_400_000;

describe("backupReminder", () => {
  it("从未备份（0/负值）为 never 且无天数", () => {
    expect(backupReminder(0, 1_000_000)).toEqual({ level: "never", days: null });
    expect(backupReminder(-5, 1_000_000)).toEqual({ level: "never", days: null });
  });

  it("整 30 天不算过期（边界：必须超过才提醒）", () => {
    const now = 1_000_000_000_000;
    expect(backupReminder(now - THIRTY_DAYS_MS, now)).toEqual({ level: "ok", days: 30 });
  });

  it("超过 30 天进入 stale", () => {
    const now = 1_000_000_000_000;
    expect(backupReminder(now - THIRTY_DAYS_MS - 1, now)).toEqual({ level: "stale", days: 30 });
    expect(backupReminder(now - 45 * DAY, now)).toEqual({ level: "stale", days: 45 });
  });

  it("30 天内为 ok；时钟回拨（未来时间）按 0 天 ok 处理", () => {
    const now = 1_000_000_000_000;
    expect(backupReminder(now - 5 * DAY, now)).toEqual({ level: "ok", days: 5 });
    expect(backupReminder(now + 3 * DAY, now)).toEqual({ level: "ok", days: 0 });
  });
});
