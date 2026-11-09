/// vault 状态机竞态回归锁（第二批 P2）：
/// ① 异步取数在途期间切走视图，含解密明文的详情页不能被硬拽回来
/// ② pinCurrent 的 invoke 必须在 setState updater 之外（StrictMode 双调）
import { StrictMode, useState, type ReactNode } from "react";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { View } from "@/App";
import type { SearchItem } from "@/plugins/types";

const { invoke, toastError, toastSuccess } = vi.hoisted(() => ({
  invoke: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess },
}));

import { useVaultActions, type VaultRecordDto } from "../useVaultActions";

const REC: VaultRecordDto = {
  title: "行云账号",
  username: "u1",
  password: "p1",
  url: "https://example.com",
  notes: "",
  pinned: 0,
  totp: "",
};

const item = (id: string): SearchItem => ({
  id: `vault:${id}`,
  title: "行云账号",
  subtitle: "",
  kind: "vault",
  target: id,
  score: 0,
});

function setup(initial: View) {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <StrictMode>{children}</StrictMode>
  );
  const { result } = renderHook(
    () => {
      const [view, setView] = useState<View>(initial);
      const actions = useVaultActions({
        view,
        setView,
        setItems: vi.fn(),
        setActive: vi.fn(),
        setQuery: vi.fn(),
        vaultTermRef: { current: "" },
        inputRef: { current: null },
      });
      return { actions, view, setView };
    },
    { wrapper }
  );
  return result;
}

const anyInvoke = (cmd: string) => {
  if (cmd === "vault_search") return Promise.resolve([]);
  return Promise.resolve("ok");
};

beforeEach(() => {
  invoke.mockReset();
  toastError.mockReset();
  toastSuccess.mockReset();
});

describe("useVaultActions 竞态守卫", () => {
  it("vault_get 在途期间切走视图，过期的详情响应被丢弃", async () => {
    let resolveGet!: (v: VaultRecordDto) => void;
    const pending = new Promise<VaultRecordDto>((r) => {
      resolveGet = r;
    });
    invoke.mockImplementation((cmd: string) =>
      cmd === "vault_get" ? pending : anyInvoke(cmd)
    );

    const result = setup({ kind: "main" } as View);
    await act(async () => {
      void result.current.actions.openDetail(item("7"));
    });
    // 用户已切到别处：这时再回来的记录带着解密明文，不能覆盖当前视图
    act(() => {
      result.current.setView({ kind: "settings" } as View);
    });
    await act(async () => {
      resolveGet(REC);
      await pending;
    });

    expect(result.current.view.kind).toBe("settings");
  });

  it("pinCurrent 在 StrictMode 下只发一次 pin 并乐观翻转置顶", async () => {
    invoke.mockImplementation((cmd: string) => anyInvoke(cmd));
    const detail = {
      kind: "detail",
      data: {
        id: 7,
        title: REC.title,
        username: REC.username,
        password: REC.password,
        url: REC.url,
        notes: "",
        pinned: 0,
        totp: "",
      },
    } as View;
    const result = setup(detail);

    await act(async () => {
      await result.current.actions.pinCurrent();
    });

    const pinCalls = invoke.mock.calls.filter(
      ([cmd, payload]) =>
        cmd === "vault_action" &&
        (payload as { action?: string } | undefined)?.action === "pin"
    );
    expect(pinCalls).toHaveLength(1);
    expect(result.current.view.kind).toBe("detail");
    if (result.current.view.kind === "detail") {
      expect(result.current.view.data.pinned).toBe(1);
    }
  });
});
