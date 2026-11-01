/// 剪贴板合并粘贴模式（自 App.tsx 拆出，审计 F3）：模式状态、勾选集、
/// 三个动作、保护域退出 effect 与窗口捕获键盘面。
/// **键盘面纪律**：effect 不传依赖数组、每次渲染重挂保闭包新鲜——hook 内
/// 保持原样（无依赖数组），items/active 经参数进入闭包、由 AppView 每渲染
/// 重挂获得最新值，与拆分前逐字节同语义（2026-10-28 三次键盘面事故同根因域，
/// 不可改成依赖数组）。isComposing 卫兵挡 IME；view 非 main 整体休眠；
/// 右键菜单的 Esc 已由 ctxMenu 捕获拦掉（stopPropagation 收不到），无重复
import { useCallback, useEffect, useState, type Dispatch, type SetStateAction, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { clipRowId, type SearchItem } from "@/plugins/types";

interface Args {
  isVault: boolean;
  view: { kind: string };
  items: SearchItem[];
  active: number;
  setActive: Dispatch<SetStateAction<number>>;
  /// 键盘导航标志（方向键置位后由滚动补位 effect 复位）
  kbNavRef: RefObject<boolean>;
}

export function useMergeMode({
  isVault,
  view,
  items,
  active,
  setActive,
  kbNavRef,
}: Args) {
  /// 剪贴板合并粘贴模式：mergeSel 收勾选的 rowid，数组保序（点选顺序 =
  /// 拼接顺序）。仅普通域剪贴板条目可勾；面板隐藏/呼出/进保护域即清
  const [mergeMode, setMergeMode] = useState(false);
  const [mergeSel, setMergeSel] = useState<number[]>([]);

  /// 面板隐藏/呼出的复位（AppView 的 hidden/shown 监听调用）
  const resetMerge = useCallback(() => {
    setMergeMode(false);
    setMergeSel([]);
  }, []);

  /// 模式开关：退出即清勾选（重进从零开始，避免残留旧意图）
  const toggleMerge = useCallback(() => {
    setMergeMode((m) => !m);
    setMergeSel([]);
  }, []);

  /// 勾选/取消一条（保序，点击与 Enter 共用）：仅文本剪贴板条目；
  /// 行号取自 target（clipRowId，与图片条目同构——内容本体不随列表过 IPC）
  const toggleClipSelection = useCallback((it: SearchItem | undefined) => {
    if (!it || it.kind !== "clip") return;
    const id = clipRowId(it);
    if (Number.isNaN(id)) return;
    setMergeSel((s) =>
      s.includes(id) ? s.filter((x) => x !== id) : [...s, id]
    );
  }, []);

  /// 执行合并：勾选顺序 = 拼接顺序。**只清勾选、不退模式**——自动粘贴关闭时
  /// 面板不收，用户往往要连着合下一批；自动粘贴开启时面板收起，zap://hidden
  /// 兜底清掉整个模式。个别条目已被清理时后端跳过并返回实际条数；全部被
  /// 清理则报错（面板不收，toast 可见）
  const performMergePaste = useCallback(async () => {
    const ids = [...mergeSel];
    setMergeSel([]);
    if (ids.length === 0) return;
    try {
      const r = await invoke<{ count: number; pasted: boolean }>(
        "clip_merge_paste",
        { ids }
      );
      const skipped = ids.length - r.count;
      const head = r.pasted
        ? `已合并粘贴 ${r.count} 条`
        : `已合并 ${r.count} 条到剪贴板`;
      toast.success(skipped > 0 ? `${head}（${skipped} 条已被清理，已跳过）` : head);
    } catch (e) {
      toast.error(String(e));
    }
  }, [mergeSel]);

  // 进入保护域（pw）即退合并模式：勾选只对普通域剪贴板列表有意义，
  // 保护域下 Enter/Ctrl+Enter 另有语义，不能被合并分支截走
  useEffect(() => {
    if (isVault && mergeMode) {
      setMergeMode(false);
      setMergeSel([]);
    }
  }, [isVault, mergeMode]);

  // 合并模式的键盘面挂窗口捕获——**不能挂面板/输入框 onKeyDown**：点击
  // 底栏按钮或条目后焦点掉到按钮/body，输入框的 onKeyDown 收不到任何键
  // （与 2026-10-22 表单 Esc 失效同根因，本次在合并功能上重犯后按同一
  // 修法收口）。捕获阶段拦下合并语义键并 stopPropagation，防输入框的
  // onKeyDown 对同一按键再处理一遍；其余按键放行不干扰打字。
  // 不传依赖数组：每次渲染重挂保闭包新鲜（VaultFormView Esc 同款）。
  useEffect(() => {
    if (!mergeMode || view.kind !== "main") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      if (e.key === "Enter" && e.ctrlKey) {
        e.preventDefault();
        e.stopPropagation();
        void performMergePaste();
      } else if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        toggleClipSelection(items[active]);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        toggleMerge();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        kbNavRef.current = true;
        setActive((a) =>
          e.key === "ArrowDown"
            ? Math.min(a + 1, Math.max(items.length - 1, 0))
            : Math.max(a - 1, 0)
        );
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return {
    mergeMode,
    mergeSel,
    resetMerge,
    toggleMerge,
    toggleClipSelection,
    performMergePaste,
  };
}
