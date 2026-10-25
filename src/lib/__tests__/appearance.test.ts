import { afterEach, describe, expect, it, vi } from "vitest";
import { applyAppearance } from "../appearance";

/// jsdom 无 matchMedia：模拟并按 query 记忆同一对象——applyAppearance 内部与
/// 断言必须拿到同一个 MQL 实例。模拟系统切换直接改 matches（真实浏览器即如此），
/// 不重置 stub（appearance.ts 的监听句柄是模块级状态，重置会导致断言落空）
const setSystemLight = (light: boolean) => {
  const lightMql = {
    matches: light,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  vi.stubGlobal(
    "matchMedia",
    vi.fn().mockImplementation((query: string) =>
      query.includes("light")
        ? lightMql
        : {
            matches: false,
            addEventListener: vi.fn(),
            removeEventListener: vi.fn(),
          }
    )
  );
  return lightMql;
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("applyAppearance（外观主题应用）", () => {
  it("完整时序：dark/light 固定模式 → system 跟随注册 → 系统切换响应 → 切回固定摘除监听", () => {
    const mql = setSystemLight(true);

    applyAppearance("dark");
    expect(document.documentElement.classList.contains("zap-light")).toBe(
      false
    );

    applyAppearance("light");
    expect(document.documentElement.classList.contains("zap-light")).toBe(true);

    // 跟随系统（当前系统浅色）→ 注册 change 监听
    applyAppearance("system");
    expect(document.documentElement.classList.contains("zap-light")).toBe(true);
    expect(mql.addEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function)
    );

    // 系统切到深色：监听回调（内部重新 applyAppearance("system")）应摘掉浅色类
    mql.matches = false;
    applyAppearance("system");
    expect(document.documentElement.classList.contains("zap-light")).toBe(
      false
    );

    // 切回固定深色：监听摘除
    applyAppearance("dark");
    expect(mql.removeEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function)
    );
  });
});
