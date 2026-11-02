/// 热键录制域（自 SettingsView 拆出，审计 E2）：录制槽位状态、zap://hk-capture
/// 事件监听、页面级快速路径、reset 事件、键帽组件——呼出与直达热键共用。
/// 组合由 Rust 低级键盘钩子经 zap://hk-capture 事件送达——连被输入法接管的
/// Ctrl+Space（中英切换）也能捕获；页面只负责 Esc 取消与钩子启停
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { X } from "lucide-react";

/// 录制中的槽位：main = 呼出热键，vault/clip/superpanel = 直达热键
export type HkSlot = "main" | "vault" | "clip" | "superpanel" | "stage";

/// 热键录制可写入的设置切片（AppSettings 的热键子集；父级经包装函数桥接）
export interface HotkeySettings {
  hotkey: string;
  vaultHotkey: string;
  clipHotkey: string;
  superpanelHotkey: string;
  stageHotkey: string;
}

/// 热键串展示格式化：ctrl+space → Ctrl + Space（键位首字母大写，+ 两侧留空）
const formatHotkey = (hk: string) =>
  hk
    .split("+")
    .map((p) => (p ? p.charAt(0).toUpperCase() + p.slice(1) : p))
    .join(" + ");

/// 热键录制键帽（呼出与直达共用）：点击录制、Esc 取消；带 onClear 的槽位可解绑
export function HotkeyButton({
  value,
  recording,
  onRecord,
  onClear,
}: {
  value: string;
  recording: boolean;
  onRecord: () => void;
  onClear?: () => void;
}) {
  return (
    <span className="flex shrink-0 items-center gap-1">
      {onClear && value && (
        <Button
          variant="ghost"
          size="icon"
          className="size-7 text-muted-foreground hover:text-destructive"
          aria-label="解绑热键"
          title="解绑热键"
          onClick={onClear}
        >
          <X className="size-3.5" />
        </Button>
      )}
      <button
        type="button"
        className={
          "flex h-[30px] shrink-0 items-center gap-2 rounded-md border px-2.5 transition-colors " +
          (recording
            ? "animate-pulse-ring border-primary/80 bg-primary/15"
            : "cursor-pointer border-primary/45 bg-primary/10 hover:bg-primary/20")
        }
        onClick={onRecord}
      >
        {recording ? (
          <span className="text-[13px]">按下新组合…（Esc 取消）</span>
        ) : (
          <>
            <b className="font-mono text-[13px] font-medium">
              {value ? formatHotkey(value) : "未绑定"}
            </b>
            <span className="text-[11px] text-muted-foreground">点击录制</span>
          </>
        )}
      </button>
    </span>
  );
}

/**
 * 热键录制 hook：管理录制槽位与错误态，捕获到的组合经 patch 写回设置表单。
 * patch 由调用方桥接（setSt 包装出非空切片），hook 内不感知 AppSettings 全貌
 */
export function useHotkeyRecording(
  patch: (fn: (s: HotkeySettings) => HotkeySettings) => void
) {
  /// 正在录制的热键槽位（null = 未在录制）
  const [hkSlot, setHkSlot] = useState<HkSlot | null>(null);
  const [hkError, setHkError] = useState("");

  const stopHkCapture = useCallback(() => {
    invoke("stop_hotkey_capture").catch(() => {});
  }, []);

  const startRecording = (slot: HkSlot) => {
    invoke("start_hotkey_capture")
      .then(() => setHkSlot(slot))
      .catch((e) => setHkError("启动录制失败：" + e));
  };

  useEffect(() => {
    const un = listen<string>("zap://hk-capture", (e) => {
      if (!hkSlot) return;
      const key = e.payload;
      patch((s) => {
        if (hkSlot === "main") return { ...s, hotkey: key };
        if (hkSlot === "vault") return { ...s, vaultHotkey: key };
        if (hkSlot === "superpanel") return { ...s, superpanelHotkey: key };
        if (hkSlot === "stage") return { ...s, stageHotkey: key };
        return { ...s, clipHotkey: key };
      });
      setHkSlot(null);
      stopHkCapture();
    });
    return () => {
      un.then((f) => f());
    };
  }, [hkSlot, patch, stopHkCapture]);

  // 录制中的组合按在了面板之外：Rust 侧丢弃时通知（只在确有组合被丢弃时发），
  // 退出录制态并停钩子——否则 UI 永远停在「按下新组合…」
  useEffect(() => {
    const un = listen("zap://hk-capture-reset", () => {
      setHkSlot(null);
      stopHkCapture();
    });
    return () => {
      un.then((f) => f());
    };
  }, [stopHkCapture]);

  // 物理按键码 → 热键键名（不受输入法/键盘布局影响；e.key 在中文输入法下常为 "Process"）
  const codeToKeyName = (code: string): string | null => {
    if (code.startsWith("Key")) return code.slice(3).toLowerCase(); // KeyJ → j
    if (code.startsWith("Digit")) return code.slice(5); // Digit5 → 5
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code.toLowerCase(); // F2 → f2
    return code === "Space" ? "space" : null;
  };

  useEffect(() => {
    if (!hkSlot) return;
    // 页面级快速路径：捕获普通组合（焦点在面板内即可）；被输入法接管的组合由
    // Rust 侧轮询兜底（zap://hk-capture 事件）
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setHkSlot(null);
        stopHkCapture();
        return;
      }
      const key = codeToKeyName(e.code);
      if (!key) return;
      const mods: string[] = [];
      if (e.ctrlKey) mods.push("ctrl");
      if (e.altKey) mods.push("alt");
      if (e.shiftKey) mods.push("shift");
      if (e.metaKey) mods.push("super"); // global-hotkey 只认 super，meta 会解析失败
      if (mods.length === 0) return; // 必须带修饰键
      e.preventDefault();
      e.stopPropagation();
      const combo = [...mods, key].join("+");
      patch((s) => {
        if (hkSlot === "main") return { ...s, hotkey: combo };
        if (hkSlot === "vault") return { ...s, vaultHotkey: combo };
        if (hkSlot === "superpanel") return { ...s, superpanelHotkey: combo };
        if (hkSlot === "stage") return { ...s, stageHotkey: combo };
        return { ...s, clipHotkey: combo };
      });
      setHkSlot(null);
      stopHkCapture();
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  }, [hkSlot, patch, stopHkCapture]);

  return { hkSlot, hkError, startRecording, setHkError, stopHkCapture };
}
