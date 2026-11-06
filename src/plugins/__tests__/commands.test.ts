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

describe("startupEntries（呼出时 Everything 入口注入 Dock）", () => {
  it("Everything 已运行：不注入条目", async () => {
    mockedInvoke.mockResolvedValue("running");
    await expect(startupEntries()).resolves.toEqual([]);
  });

  it("已安装未运行：注入启用条目（提示启动）", async () => {
    mockedInvoke.mockResolvedValue("installed_not_running");
    const entries = await startupEntries();
    expect(entries).toHaveLength(1);
    expect(entries[0].item.kind).toBe("cmd");
    expect(entries[0].item.target).toBe("everything_setup");
    expect(entries[0].item.subtitle).toContain("启动");
    expect(entries[0].label).toBe("Everything");
  });

  it("未安装：注入下载条目（提示自动下载）", async () => {
    mockedInvoke.mockResolvedValue("not_installed");
    const entries = await startupEntries();
    expect(entries).toHaveLength(1);
    expect(entries[0].item.subtitle).toContain("下载");
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

  it("Everything 安装失败：失败条目带原因，hold 与 busy 仍释放", async () => {
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
    expect(ctx.setBusy).toHaveBeenLastCalledWith(false);
    expect(mockedInvoke).toHaveBeenCalledWith("set_hold_panel", { hold: false });
  });

  it("面板隐藏后放弃等待：在途回执不回写，hold 与 busy 仍释放", async () => {
    const ctx = makeCtx();
    let stale = false;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "everything_setup") return Promise.resolve("running");
      return Promise.resolve(null);
    });
    const pending = runCommand("everything_setup", {
      ...ctx,
      isStale: () => stale,
    });
    stale = true; // 模拟等待期间用户按 Esc 隐藏面板
    await pending;
    // 面板已隐藏 = 整条流程作废：连占位行都不写，不污染下次呼出的列表
    expect(ctx.setItems).not.toHaveBeenCalled();
    expect(ctx.setBusy).toHaveBeenLastCalledWith(false);
    // hold 尚未设起（设 hold 在状态探测之后），无需释放
    expect(mockedInvoke).not.toHaveBeenCalledWith("set_hold_panel", {
      hold: true,
    });
  });

  it("已安装：占位文案说启动、不说下载", async () => {
    const ctx = makeCtx();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "everything_status")
        return Promise.resolve("installed_not_running");
      if (cmd === "everything_setup") return Promise.resolve("running");
      return Promise.resolve(null);
    });
    await runCommand("everything_setup", ctx);
    const first = ctx.setItems.mock.calls[0][0] as {
      id: string;
      subtitle: string;
    }[];
    expect(first[0].id).toBe("setup-busy");
    expect(first[0].subtitle).toContain("启动");
    expect(first[0].subtitle).not.toContain("下载");
  });

  it("未安装：占位文案说下载约 2MB", async () => {
    const ctx = makeCtx();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "everything_status") return Promise.resolve("not_installed");
      if (cmd === "everything_setup") return Promise.resolve("running");
      return Promise.resolve(null);
    });
    await runCommand("everything_setup", ctx);
    const first = ctx.setItems.mock.calls[0][0] as {
      id: string;
      subtitle: string;
    }[];
    expect(first[0].subtitle).toContain("下载");
  });

  it("HTTP 未就绪：不当失败，单独文案", async () => {
    const ctx = makeCtx();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "everything_setup") return Promise.resolve("running_no_http");
      return Promise.resolve(null);
    });
    await runCommand("everything_setup", ctx);
    const last = ctx.setItems.mock.calls[
      ctx.setItems.mock.calls.length - 1
    ]?.[0] as { id: string; title: string }[];
    expect(last[0].id).toBe("setup-http-pending");
    expect(last[0].title).toContain("HTTP");
  });
});
