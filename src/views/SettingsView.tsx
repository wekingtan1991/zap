/// 设置页（从 App.tsx 迁出的自包含视图）：设置加载/保存、热键录制、
/// 各分区渲染。更新区消费 AppView 传入的 useUpdater 状态机。
/// 挂载即拉取设置，卸载即停热键钩子——开关状态由父级 settingsOpen 决定
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  ArrowLeft,
  Clipboard,
  Download,
  KeyRound,
  Loader2,
  Puzzle,
  RefreshCw,
  RotateCw,
  Search,
  Settings as SettingsIcon,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { Updater } from "./useUpdater";

export interface PluginState {
  id: string;
  title: string;
  hint: string;
  enabled: boolean;
}

export interface AppSettings {
  hotkey: string;
  clipEnabled: boolean;
  clipMax: number;
  pwdClearSecs: number;
  autostart: boolean;
  everythingAutostart: boolean;
  /// 可开关插件清单（后端 meta 下发，设置页「插件」分区渲染）
  plugins: PluginState[];
}

/// 热键串展示格式化：ctrl+space → Ctrl + Space（键位首字母大写，+ 两侧留空）
const formatHotkey = (hk: string) =>
  hk
    .split("+")
    .map((p) => (p ? p.charAt(0).toUpperCase() + p.slice(1) : p))
    .join(" + ");

interface Props {
  updater: Updater;
  onKeyDown: (e: React.KeyboardEvent) => void;
  focusRef: (el: HTMLDivElement | null) => void;
  onClose: () => void;
}

export function SettingsView({ updater, onKeyDown, focusRef, onClose }: Props) {
  const [st, setSt] = useState<AppSettings | null>(null);
  const [appVersion, setAppVersion] = useState("");
  const [hkRecording, setHkRecording] = useState(false);
  const [hkError, setHkError] = useState("");
  const [saveMsg, setSaveMsg] = useState("");

  useEffect(() => {
    invoke<AppSettings>("get_app_settings")
      .then(setSt)
      .catch(() => {});
    // 版本号异步读取（tauri.conf.json 的 version），失败时留空
    getVersion()
      .then(setAppVersion)
      .catch(() => {});
    // 卸载即停热键钩子（正常关闭由 Esc/按钮触发，面板失焦隐没时兜底）
    return () => {
      invoke("stop_hotkey_capture").catch(() => {});
    };
  }, []);

  const stopHkCapture = useCallback(() => {
    invoke("stop_hotkey_capture").catch(() => {});
  }, []);

  // ---------- 热键录制（原 App.tsx 机制原样迁入） ----------
  // 组合由 Rust 低级键盘钩子经 zap://hk-capture 事件送达——连被输入法接管的
  // Ctrl+Space（中英切换）也能捕获；页面只负责 Esc 取消与钩子启停
  useEffect(() => {
    const un = listen<string>("zap://hk-capture", (e) => {
      setSt((s) => (s ? { ...s, hotkey: e.payload } : s));
      setHkRecording(false);
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
    if (!hkRecording) return;
    // 页面级快速路径：捕获普通组合（焦点在面板内即可）；被输入法接管的组合由
    // Rust 侧轮询兜底（zap://hk-capture 事件）
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setHkRecording(false);
        stopHkCapture();
        return;
      }
      const key = codeToKeyName(e.code);
      if (!key) return;
      const mods: string[] = [];
      if (e.ctrlKey) mods.push("ctrl");
      if (e.altKey) mods.push("alt");
      if (e.shiftKey) mods.push("shift");
      if (e.metaKey) mods.push("meta");
      if (mods.length === 0) return; // 必须带修饰键
      e.preventDefault();
      e.stopPropagation();
      setSt((s) => (s ? { ...s, hotkey: [...mods, key].join("+") } : s));
      setHkRecording(false);
      stopHkCapture();
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  }, [hkRecording, stopHkCapture]);

  const saveSettings = async () => {
    if (!st) return;
    try {
      const res = await invoke<{ hotkeyError: string | null }>("save_settings", {
        hotkey: st.hotkey,
        clipEnabled: st.clipEnabled,
        clipMax: st.clipMax,
        pwdClearSecs: st.pwdClearSecs,
        everythingAutostart: st.everythingAutostart,
        plugins: st.plugins.map((p) => ({ id: p.id, enabled: p.enabled })),
      });
      if (res.hotkeyError) {
        setHkError(res.hotkeyError);
        setSt(await invoke<AppSettings>("get_app_settings")); // 回显实际生效值
      } else {
        setHkError("");
        setSaveMsg("已保存 ✓");
        setTimeout(() => setSaveMsg(""), 2000);
      }
    } catch (e) {
      setHkError(String(e));
    }
  };

  const {
    status: updateStatus,
    version: updateVersion,
    progress: updateProgress,
    msg: updateMsg,
  } = updater;

  if (!st) {
    // 设置加载中的一帧空壳（get_app_settings 实际不会失败，罕见网络态兜底）
    return (
      <div
        className="zap-panel outline-none"
        tabIndex={-1}
        ref={focusRef}
        onKeyDown={onKeyDown}
      />
    );
  }

  return (
    <div
      className="zap-panel vault outline-none"
      tabIndex={-1}
      ref={focusRef}
      onKeyDown={onKeyDown}
    >
      <div className="zap-input-row">
        <span className="zap-logo">
          <SettingsIcon className="size-4 text-primary" />
        </span>
        <span className="zap-vault-tag">设置</span>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={onClose}
        >
          <ArrowLeft className="size-3" />
          返回（Esc）
        </Button>
      </div>
      <div className="flex-1 overflow-y-auto px-[22px] pb-1.5 pt-2.5">
        <div className="mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
          <Zap className="size-3 text-primary/75" />
          启动
        </div>
        <div className="flex min-h-[38px] items-center justify-between gap-3">
          <span className="text-[13px]">呼出热键</span>
          <button
            type="button"
            className={
              "flex h-[30px] shrink-0 items-center gap-2 rounded-md border px-2.5 transition-colors " +
              (hkRecording
                ? "animate-pulse-ring border-primary/80 bg-primary/15"
                : "cursor-pointer border-primary/45 bg-primary/10 hover:bg-primary/20")
            }
            onClick={() => {
              invoke("start_hotkey_capture")
                .then(() => setHkRecording(true))
                .catch((e) => setHkError("启动录制失败：" + e));
            }}
          >
            {hkRecording ? (
              <span className="text-[12.5px]">按下新组合…（Esc 取消）</span>
            ) : (
              <>
                <b className="font-mono text-[12.5px] font-medium">
                  {formatHotkey(st.hotkey)}
                </b>
                <span className="text-[11px] text-muted-foreground">
                  点击录制
                </span>
              </>
            )}
          </button>
        </div>
        {hkError && (
          <div className="animate-shake pb-1 text-[11px] text-destructive">
            {hkError}
          </div>
        )}
        <div className="flex min-h-[38px] items-center justify-between gap-3">
          <Label
            htmlFor="set-autostart"
            className="text-[13px] font-normal text-foreground"
          >
            开机自启
          </Label>
          <Checkbox
            id="set-autostart"
            checked={st.autostart}
            onCheckedChange={async (v) => {
              const on = v === true;
              setSt((s) => (s ? { ...s, autostart: on } : s));
              try {
                await invoke("autostart_set", { enable: on });
              } catch {
                setSt((s) => (s ? { ...s, autostart: !on } : s));
              }
            }}
          />
        </div>
        <div className="mt-3 border-t border-white/5 pt-3">
          <div className="mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Clipboard className="size-3 text-primary/75" />
            剪贴板
          </div>
          <div className="flex min-h-[38px] items-center justify-between gap-3">
            <Label
              htmlFor="set-clip"
              className="text-[13px] font-normal text-foreground"
            >
              记录剪贴板历史
            </Label>
            <Checkbox
              id="set-clip"
              checked={st.clipEnabled}
              onCheckedChange={(v) =>
                setSt((s) => (s ? { ...s, clipEnabled: v === true } : s))
              }
            />
          </div>
          <div className="flex min-h-[38px] items-center justify-between gap-3">
            <Label className="text-[13px] font-normal text-foreground">
              保留条数
            </Label>
            <Select
              value={String(st.clipMax)}
              disabled={!st.clipEnabled}
              onValueChange={(v) =>
                setSt((s) => (s ? { ...s, clipMax: Number(v) } : s))
              }
            >
              <SelectTrigger className="h-[30px] w-[126px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="100">100</SelectItem>
                <SelectItem value="500">500</SelectItem>
                <SelectItem value="1000">1000</SelectItem>
                <SelectItem value="5000">5000</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
        <div className="mt-3 border-t border-white/5 pt-3">
          <div className="mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <KeyRound className="size-3 text-primary/75" />
            密码安全
          </div>
          <div className="flex min-h-[38px] items-center justify-between gap-3">
            <Label className="text-[13px] font-normal text-foreground">
              复制后清空剪贴板
            </Label>
            <Select
              value={String(st.pwdClearSecs)}
              onValueChange={(v) =>
                setSt((s) => (s ? { ...s, pwdClearSecs: Number(v) } : s))
              }
            >
              <SelectTrigger className="h-[30px] w-[126px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="0">关闭</SelectItem>
                <SelectItem value="30">30 秒</SelectItem>
                <SelectItem value="45">45 秒</SelectItem>
                <SelectItem value="60">60 秒</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
        <div className="mt-3 border-t border-white/5 pt-3">
          <div className="mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Search className="size-3 text-primary/75" />
            Everything
          </div>
          <div className="flex min-h-[38px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <Label
                htmlFor="set-ev"
                className="text-[13px] font-normal text-foreground"
              >
                自动启动 Everything
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                未运行时自动拉起；不会自动下载
              </span>
            </span>
            <Checkbox
              id="set-ev"
              checked={st.everythingAutostart}
              onCheckedChange={(v) =>
                setSt((s) =>
                  s ? { ...s, everythingAutostart: v === true } : s
                )
              }
            />
          </div>
        </div>
        <div className="mt-3 border-t border-white/5 pt-3">
          <div className="mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Puzzle className="size-3 text-primary/75" />
            插件
          </div>
          {st.plugins.length === 0 ? (
            <div className="py-2 text-[11px] text-muted-foreground">
              暂无可开关插件
            </div>
          ) : (
            st.plugins.map((p) => (
              <div
                key={p.id}
                className="flex min-h-[38px] items-center justify-between gap-3"
              >
                <span className="flex min-w-0 flex-col gap-px">
                  <Label
                    htmlFor={"set-plugin-" + p.id}
                    className="text-[13px] font-normal text-foreground"
                  >
                    {p.title}
                  </Label>
                  <span className="text-[11px] leading-snug text-muted-foreground">
                    {p.hint}
                  </span>
                </span>
                <Checkbox
                  id={"set-plugin-" + p.id}
                  checked={p.enabled}
                  onCheckedChange={(v) =>
                    setSt((s) =>
                      s
                        ? {
                            ...s,
                            plugins: s.plugins.map((x) =>
                              x.id === p.id ? { ...x, enabled: v === true } : x
                            ),
                          }
                        : s
                    )
                  }
                />
              </div>
            ))
          )}
        </div>
        <div className="mt-3 border-t border-white/5 pt-3">
          <div className="mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <RefreshCw className="size-3 text-primary/75" />
            软件更新
          </div>
          <div className="flex min-h-[38px] items-center justify-between gap-3">
            <span className="flex min-w-0 items-center gap-2.5">
              <span className="shrink-0 text-[13px]">当前版本</span>
              <b className="shrink-0 text-[13px] font-medium text-primary/90">
                v{appVersion || "…"}
              </b>
              <span
                className="flex min-w-0 items-center gap-2"
                aria-live="polite"
              >
                {updateStatus === "idle" && updateMsg && (
                  <span
                    className={
                      "truncate text-xs " +
                      (updateMsg.kind === "ok"
                        ? "text-success"
                        : "text-destructive")
                    }
                  >
                    {updateMsg.text}
                  </span>
                )}
                {updateStatus === "checking" && (
                  <span className="truncate text-xs text-muted-foreground">
                    正在检查更新…
                  </span>
                )}
                {updateStatus === "available" && (
                  <span className="truncate text-xs text-muted-foreground">
                    发现新版本{" "}
                    <b className="font-medium text-primary/90">
                      v{updateVersion}
                    </b>
                  </span>
                )}
                {updateStatus === "downloading" && (
                  <>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      正在下载更新
                    </span>
                    <span className="h-1 w-[90px] shrink-0 overflow-hidden rounded-full bg-muted">
                      <span
                        className="block h-full rounded-full bg-primary transition-[width] duration-150"
                        style={{ width: `${updateProgress}%` }}
                      />
                    </span>
                  </>
                )}
                {updateStatus === "ready" && (
                  <span className="truncate text-xs text-muted-foreground">
                    更新已就绪，重启 zap 完成安装
                  </span>
                )}
              </span>
            </span>
            {updateStatus === "idle" && (
              <Button
                variant="outline"
                size="sm"
                className="shrink-0"
                onClick={() => void updater.checkForUpdate()}
              >
                检查更新
              </Button>
            )}
            {updateStatus === "checking" && (
              <Button variant="outline" size="sm" className="shrink-0" disabled>
                <Loader2 className="size-3.5 animate-spin" />
                检查中
              </Button>
            )}
            {updateStatus === "downloading" && (
              <Button variant="outline" size="sm" className="shrink-0" disabled>
                {updateProgress}%
              </Button>
            )}
            {updateStatus === "available" && (
              <Button
                variant="outline"
                size="sm"
                className="shrink-0 border-primary/50 text-primary hover:text-primary"
                onClick={() => void updater.downloadUpdate()}
              >
                <Download className="size-3.5" />
                下载并安装
              </Button>
            )}
            {updateStatus === "ready" && (
              <Button
                size="sm"
                className="shrink-0"
                onClick={() => void relaunch()}
              >
                <RotateCw className="size-3.5" />
                重启以完成更新
              </Button>
            )}
          </div>
        </div>
      </div>
      <div className="flex shrink-0 items-center justify-end gap-2 border-t border-border px-[22px] py-1.5">
        {saveMsg && (
          <span className="shrink-0 animate-fade-in text-xs text-success">
            {saveMsg}
          </span>
        )}
        <Button size="sm" className="shrink-0" onClick={() => void saveSettings()}>
          保存
        </Button>
      </div>
    </div>
  );
}
