import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { runCommand, startupEntries, type CommandCtx } from "../commands";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);

type TestCtx = CommandCtx & {
  setBusy: ReturnType<typeof vi.fn>;
  setItems: ReturnType<typeof vi.fn>;
};

const makeCtx = (): TestCtx =>
  ({
    busyRef: { current: false },
    setBusy: vi.fn(),
    setItems: vi.fn(),
  }) as unknown as TestCtx;

beforeEach(() => {
  mockedInvoke.mockReset();
});

describe("startupEntries（呼出时 Everything 状态注入）", () => {
  it("Everything 已运行：不注入条目", async () => {
    mockedInvoke.mockResolvedValue("running");
    await expect(startupEntries()).resolves.toEqual([]);
  });

  it("已安装未运行：注入启用条目（提示启动）", async () => {
    mockedInvoke.mockResolvedValue("installed_not_running");
    const entries = await startupEntries();
    expect(entries).toHaveLength(1);
    expect(entries[0].kind).toBe("cmd");
    expect(entries[0].target).toBe("everything_setup");
    expect(entries[0].subtitle).toContain("启动");
  });

  it("未安装：注入下载条目（提示自动下载）", async () => {
    mockedInvoke.mockResolvedValue("not_installed");
    const entries = await startupEntries();
    expect(entries).toHaveLength(1);
    expect(entries[0].subtitle).toContain("下载");
  });

  it("状态查询失败：静默不注入", async () => {
    mockedInvoke.mockRejectedValue(new Error("boom"));
    await expect(startupEntries()).resolves.toEqual([]);
  });
});

describe("runCommand（cmd 条目执行器）", () => {
  it("未知 target 忽略", async () => {
    await expect(runCommand("nope", makeCtx())).resolves.toBeUndefined();
  });

  it("Everything 安装成功：占位 → 成功条目，期间保持面板可见", async () => {
    const ctx = makeCtx();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "everything_setup") return Promise.resolve("running");
      return Promise.resolve(null); // set_hold_panel
    });
    await runCommand("everything_setup", ctx);
    expect(ctx.setBusy).toHaveBeenNthCalledWith(1, true);
    expect(ctx.setBusy).toHaveBeenLastCalledWith(false);
    const frames = ctx.setItems.mock.calls.map((c) => c[0] as { id: string }[]);
    expect(frames[0][0].id).toBe("setup-busy");
    expect(frames[frames.length - 1][0].id).toBe("setup-done");
    // UAC 期间不触发失焦即隐
    expect(mockedInvoke).toHaveBeenCalledWith("set_hold_panel", { hold: true });
    expect(mockedInvoke).toHaveBeenCalledWith("set_hold_panel", { hold: false });
  });

  it("Everything 安装失败：失败条目带原因", async () => {
    const ctx = makeCtx();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "everything_setup") return Promise.reject(new Error("UAC"));
      return Promise.resolve(null);
    });
    await runCommand("everything_setup", ctx);
    const last = ctx.setItems.mock.calls[
      ctx.setItems.mock.calls.length - 1
    ]?.[0] as {
      id: string;
      subtitle: string;
    }[];
    expect(last[0].id).toBe("setup-fail");
    expect(last[0].subtitle).toContain("UAC");
  });
});
