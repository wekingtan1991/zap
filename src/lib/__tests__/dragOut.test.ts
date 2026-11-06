import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { isDragOutActive, startDragOut } from "../dragOut";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);

/// invoke().catch().finally() 的回调在微任务里解除 sessionPending / click 抑制器
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

function press(row: HTMLElement, x = 100, y = 100): void {
  startDragOut(
    { button: 0, clientX: x, clientY: y, target: row, currentTarget: row },
    "clip",
    42
  );
}

function move(x: number, y: number): void {
  window.dispatchEvent(
    new MouseEvent("mousemove", { clientX: x, clientY: y, bubbles: true })
  );
}

beforeEach(async () => {
  mockedInvoke.mockReset();
  mockedInvoke.mockResolvedValue({ effect: 1 });
  // 清掉上一用例可能残留的预备期监听
  window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  await settle();
  await vi.waitFor(() => expect(isDragOutActive()).toBe(false));
  document.body.innerHTML = "";
});

describe("startDragOut 意图判定", () => {
  it("元素级 blur（搜索框失焦）不掐掉第一次拖动", async () => {
    const row = document.createElement("div");
    const input = document.createElement("input");
    document.body.append(row, input);

    // 先证明 blur 虽不冒泡、捕获阶段确实能到 window——否则这条用例是假通过
    const probe = vi.fn();
    window.addEventListener("blur", probe, true);

    press(row);
    // 呼出后焦点恒在搜索框；按结果行使 input 失焦，blur 捕获阶段经过 window
    input.dispatchEvent(new Event("blur", { bubbles: false }));
    expect(probe).toHaveBeenCalledTimes(1);
    window.removeEventListener("blur", probe, true);

    move(120, 100);
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(mockedInvoke).toHaveBeenCalledWith("drag_out_begin", {
      kind: "clip",
      rowid: 42,
    });
    await settle();
  });

  it("窗口自身失焦仍清态：预备期 blur 后移动不 invoke", () => {
    const row = document.createElement("div");
    document.body.append(row);

    // 记录窗口级 blur 的 target：它不能是页面元素/Document，否则类型判断会误放行
    let target: EventTarget | null = null;
    const probe = (ev: Event) => {
      target = ev.target;
    };
    window.addEventListener("blur", probe, true);

    press(row);
    window.dispatchEvent(new Event("blur"));
    window.removeEventListener("blur", probe, true);
    expect(target).not.toBeInstanceOf(Element);
    expect(target).not.toBeInstanceOf(Document);

    move(120, 100);
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("丢失 mouseup 后新按下先清旧监听，不会用陈旧起点误起拖", async () => {
    const row = document.createElement("div");
    document.body.append(row);

    press(row, 100, 100); // 第一组：起点 (100,100)，故意不派发 mouseup
    press(row, 300, 300); // 新按下：起点 (300,300)，应注销上一组
    move(305, 300); // 相对新起点仅 5px；旧监听会误判成 205px 而起拖
    expect(mockedInvoke).not.toHaveBeenCalled();

    move(320, 300); // 20px，正常起拖
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    await settle();
  });

  it("6px 内松手 = 干净单击；右键不参与意图", () => {
    const row = document.createElement("div");
    document.body.append(row);

    press(row);
    move(104, 100);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    expect(mockedInvoke).not.toHaveBeenCalled();

    startDragOut(
      {
        button: 2,
        clientX: 100,
        clientY: 100,
        target: row,
        currentTarget: row,
      },
      "clip",
      42
    );
    move(150, 100);
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});
