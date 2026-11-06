/// 命令型条目（kind "cmd"）：target → 执行器注册表。
/// cmd 条目不走后端 open_item，激活时按 target 在这里查执行器；
/// 工具 Dock 的入口拉取与面板呼出时的状态注入（Everything 未运行追加启用
/// 条目）也一并收敛于此——两者都只喂 Dock，不再往结果列表里塞入口行
import { invoke } from "@tauri-apps/api/core";
import type { DockEntry, SearchItem } from "./types";

export interface CommandCtx {
  busyRef: { current: boolean };
  setBusy: (b: boolean) => void;
  setItems: (items: SearchItem[]) => void;
}

/// 安装/启动 Everything 并等待就绪；期间保持面板可见（UAC 抢焦点不触发失焦即隐）
async function runEverythingSetup({
  busyRef,
  setBusy,
  setItems,
}: CommandCtx): Promise<void> {
  if (busyRef.current) return;
  setBusy(true);
  setItems([
    {
      id: "setup-busy",
      title: "正在准备 Everything…",
      subtitle: "下载约 2MB 并自动配置；若弹出 UAC 请点“是”",
      kind: "info",
      target: "",
      score: 0,
    },
  ]);
  await invoke("set_hold_panel", { hold: true }).catch(() => {});
  const result = await invoke<string>("everything_setup").catch(
    (e) => "error: " + e
  );
  await invoke("set_hold_panel", { hold: false }).catch(() => {});
  if (result === "running") {
    setItems([
      {
        id: "setup-done",
        title: "全盘搜索已启用 ✓",
        subtitle: "输入任意文件名即可搜索全盘，Esc 关闭后立即生效",
        kind: "info",
        target: "",
        score: 0,
      },
    ]);
  } else {
    setItems([
      {
        id: "setup-fail",
        title: "启用未完成",
        subtitle: String(result),
        kind: "info",
        target: "",
        score: 0,
      },
    ]);
  }
  setBusy(false);
}

const COMMANDS: Record<string, (ctx: CommandCtx) => Promise<void>> = {
  everything_setup: runEverythingSetup,
};

/// cmd 条目激活入口；未知 target 忽略（与旧行为一致：目前仅 Everything 有执行器）
export function runCommand(target: string, ctx: CommandCtx): Promise<void> {
  return COMMANDS[target]?.(ctx) ?? Promise.resolve();
}

/// 常驻工具 Dock 的插件入口清单：后端按启用插件收拢 fallback_entry()，
/// 与 search 结果完全分离。挂载 / 呼出 / 设置页插件开关变化后各拉一次
export async function toolEntries(): Promise<DockEntry[]> {
  try {
    return await invoke<DockEntry[]>("tool_entries");
  } catch {
    return [];
  }
}

/// 面板呼出时的状态注入：Everything 未运行则返回启用条目（状态查询失败不影响
/// Dock）。语义与筛选口径原样保留，仅落点从结果列表迁到 Dock 细条
export async function startupEntries(): Promise<DockEntry[]> {
  try {
    const st = await invoke<string>("everything_status");
    if (st === "running") return [];
    return [
      {
        item: {
          id: "cmd:everything",
          title: "启用全盘文件搜索（Everything）",
          subtitle:
            st === "installed_not_running"
              ? "启动已就绪的 Everything · 弹出 UAC 请点“是”"
              : "自动下载并配置，约 2MB · 弹出 UAC 请点“是”",
          kind: "cmd",
          target: "everything_setup",
          score: 0,
        },
        label: "Everything",
        words: [],
      },
    ];
  } catch {
    return [];
  }
}

/// Everything 未运行：后台自动启动（后端按设置与节流决定是否真的动作），
/// 就绪后回调以便从 Dock 移除这条一次性启用入口
export function kickEverythingEnsure(onReady: () => void): void {
  invoke<string>("everything_ensure")
    .then((after) => {
      if (after === "running") onReady();
    })
    .catch(() => {});
}
