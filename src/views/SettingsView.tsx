/// 设置页（从 App.tsx 迁出的自包含视图）：设置加载/保存、热键录制、
/// 各分区渲染。更新区消费 AppView 传入的 useUpdater 状态机。
/// 挂载即拉取设置，卸载即停热键钩子——开关状态由父级 view 状态机决定
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";
import {
  Archive,
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  Clipboard,
  Database,
  Download,
  Globe,
  Inbox,
  KeyRound,
  Loader2,
  Palette,
  Plus,
  Puzzle,
  RefreshCw,
  RotateCw,
  Search,
  Settings as SettingsIcon,
  X,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { Updater } from "./useUpdater";
import { useEscCapture } from "@/hooks/useEscCapture";
import {
  countImportedSettings,
  mergeImportedConfig,
  type ConfigFilePayload,
} from "@/lib/configfile";
import {
  applyAppearance,
  applyPanelTransparency,
  type Appearance,
} from "@/lib/appearance";

export interface PluginState {
  id: string;
  title: string;
  hint: string;
  enabled: boolean;
  /// 搜索入口词（后端 PluginMeta.entries 声明化下发）
  entries: string[];
  /// 入口词冲突（与其他插件或搜索捷径关键词重合，后端现算下发）
  conflicts: string[];
}

export interface AppSettings {
  hotkey: string;
  /// 账号库直达热键（空串 = 未绑定）
  vaultHotkey: string;
  /// 快速粘贴直达热键（空串 = 未绑定）
  clipHotkey: string;
  /// 超级面板直达热键（空串 = 未绑定）
  superpanelHotkey: string;
  /// 中转站直达热键（空串 = 未绑定）
  stageHotkey: string;
  clipEnabled: boolean;
  clipMax: number;
  /// 剪贴板条目选中后自动粘贴到原窗口
  clipAutoPaste: boolean;
  /// 中转站未固定条目容量上限
  stageMax: number;
  /// 全局拖拽落区条开关
  stagedropEnabled: boolean;
  /// 拖入落区条松手后自动呼出中转站面板
  stagedropOpenPanel: boolean;
  pwdClearSecs: number;
  autostart: boolean;
  everythingAutostart: boolean;
  /// 外观模式（dark/light/system）
  appearance: string;
  /// 透明面板（亚克力窗口特效）
  panelTransparency: boolean;
  /// 可开关插件清单（后端 meta 下发，设置页「插件」分区渲染）
  plugins: PluginState[];
}

/// 备份条目（后端 backup::BackupEntry 的线协议镜像）
interface BackupEntry {
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
  pinned: number;
}

/// 搜索捷径（后端 shortcuts::Shortcut 的线协议镜像）：
/// keyword 触发词 / name 展示名 / url 含 {query} 占位的模板
interface Shortcut {
  keyword: string;
  name: string;
  url: string;
}

/// 解密预览（后端 backup::BackupPreview）：条目 + 疑似重复标记
interface BackupPreview {
  entries: BackupEntry[];
  duplicates: boolean[];
}

/// 热键串展示格式化：ctrl+space → Ctrl + Space（键位首字母大写，+ 两侧留空）
const formatHotkey = (hk: string) =>
  hk
    .split("+")
    .map((p) => (p ? p.charAt(0).toUpperCase() + p.slice(1) : p))
    .join(" + ");

/// 录制中的槽位：main = 呼出热键，vault/clip/superpanel = 直达热键
type HkSlot = "main" | "vault" | "clip" | "superpanel" | "stage";

/// 热键录制键帽（呼出与直达共用）：点击录制、Esc 取消；带 onClear 的槽位可解绑
function HotkeyButton({
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
          <span className="text-[12.5px]">按下新组合…（Esc 取消）</span>
        ) : (
          <>
            <b className="font-mono text-[12.5px] font-medium">
              {value ? formatHotkey(value) : "未绑定"}
            </b>
            <span className="text-[11px] text-muted-foreground">点击录制</span>
          </>
        )}
      </button>
    </span>
  );
}

interface Props {
  updater: Updater;
  focusRef: (el: HTMLDivElement | null) => void;
  onClose: () => void;
}

export function SettingsView({ updater, focusRef, onClose }: Props) {
  const [st, setSt] = useState<AppSettings | null>(null);
  const [appVersion, setAppVersion] = useState("");
  /// 正在录制的热键槽位（null = 未在录制）
  const [hkSlot, setHkSlot] = useState<HkSlot | null>(null);
  const [hkError, setHkError] = useState("");
  /// 直达热键保存错误（与主热键错误分开呈现）
  const [directErr, setDirectErr] = useState("");
  const [saveMsg, setSaveMsg] = useState("");
  /// 搜索捷径清单：随「保存」整单提交（校验失败留页修正）
  const [scList, setScList] = useState<Shortcut[]>([]);
  const [scErr, setScErr] = useState("");

  // ---------- 账号库备份（加密导出/恢复，弹窗承载流程，动作即时执行不随「保存」） ----------
  // 导出：弹窗填口令（二次确认）→ save 对话框 → 加密落盘
  // 导入：open 对话框选文件 → 弹窗输口令 → 解密预览 → 确认恢复（追加写入）
  // 文件对话框会抢主窗口焦点触发「失焦即隐」：期间一律 set_hold_panel 保前台
  const [bkDialog, setBkDialog] = useState<
    | { kind: "export" }
    | { kind: "import"; path: string; stage: "pass" | "confirm" }
    | null
  >(null);
  const [bkPass, setBkPass] = useState("");
  const [bkPass2, setBkPass2] = useState("");
  const [bkBusy, setBkBusy] = useState(false);
  const [bkErr, setBkErr] = useState("");
  const [bkPreview, setBkPreview] = useState<BackupPreview | null>(null);
  /// 恢复模式：跳过疑似重复（默认推荐）/ 全部追加
  const [bkSkip, setBkSkip] = useState(true);

  const closeBkDialog = () => {
    setBkDialog(null);
    setBkPass("");
    setBkPass2("");
    setBkErr("");
    setBkPreview(null);
    setBkSkip(true);
  };

  const doExport = async () => {
    setBkBusy(true);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    try {
      const path = await save({
        defaultPath: `zap-vault-${new Date().toISOString().slice(0, 10)}.zapvault`,
        filters: [{ name: "zap 账号备份", extensions: ["zapvault"] }],
      });
      if (!path) return; // 取消选位：留在弹窗继续
      const n = await invoke<number>("vault_backup_export", {
        path,
        passphrase: bkPass,
      });
      toast.success(`已导出 ${n} 条账号`);
      closeBkDialog();
    } catch (e) {
      setBkErr(String(e));
      toast.error(String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
      setBkBusy(false);
    }
  };

  const startImport = async () => {
    setBkBusy(true);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "zap 账号备份", extensions: ["zapvault"] }],
      });
      if (path && typeof path === "string") {
        setBkDialog({ kind: "import", path, stage: "pass" });
        setBkErr("");
      }
    } catch (e) {
      toast.error(String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
      setBkBusy(false);
    }
  };

  const doDecrypt = async () => {
    if (!bkDialog || bkDialog.kind !== "import") return;
    setBkBusy(true);
    try {
      const preview = await invoke<BackupPreview>("vault_backup_decrypt", {
        path: bkDialog.path,
        passphrase: bkPass,
      });
      setBkPreview(preview);
      setBkSkip(true);
      setBkDialog({ ...bkDialog, stage: "confirm" });
      setBkErr("");
    } catch (e) {
      // 口令错误等留在弹窗内重输，不 toast（输入焦点还在原处）
      setBkErr(String(e));
    } finally {
      setBkBusy(false);
    }
  };

  const doRestore = async () => {
    if (!bkPreview) return;
    setBkBusy(true);
    try {
      const [written, skipped] = await invoke<[number, number]>(
        "vault_backup_restore",
        { entries: bkPreview.entries, skipDuplicates: bkSkip }
      );
      toast.success(
        skipped > 0
          ? `已恢复 ${written} 条账号，跳过重复 ${skipped} 条`
          : `已恢复 ${written} 条账号`
      );
      closeBkDialog();
    } catch (e) {
      setBkErr("恢复失败：" + String(e));
      toast.error("恢复失败：" + String(e));
    } finally {
      setBkBusy(false);
    }
  };

  // ---------- 配置文件（设置+插件开关+捷径打包导出；导入回填表单，随「保存」生效） ----------
  const [cfgBusy, setCfgBusy] = useState(false);

  const doConfigExport = async () => {
    setCfgBusy(true);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    try {
      const path = await save({
        defaultPath: `zap-config-${new Date().toISOString().slice(0, 10)}.json`,
        filters: [{ name: "zap 配置文件", extensions: ["json"] }],
      });
      if (!path) return;
      await invoke("config_export", { path });
      toast.success("配置已导出（不含账号库数据）");
    } catch (e) {
      toast.error(String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
      setCfgBusy(false);
    }
  };

  const doConfigImport = async () => {
    setCfgBusy(true);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "zap 配置文件", extensions: ["json"] }],
      });
      if (!path || typeof path !== "string") return;
      const file = await invoke<ConfigFilePayload>("config_import", { path });
      setSt((s) => (s ? mergeImportedConfig(s, file) : s));
      setScList(file.shortcuts ?? []);
      const n = countImportedSettings(file);
      toast.success(
        n > 0
          ? `已载入配置（设置 ${n} 项 · 捷径 ${(file.shortcuts ?? []).length} 条），核对后点「保存」生效`
          : `已载入捷径 ${(file.shortcuts ?? []).length} 条，核对后点「保存」生效`
      );
    } catch (e) {
      toast.error(String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
      setCfgBusy(false);
    }
  };

  useEffect(() => {
    // 归一化：半新半旧会话里旧后端不下发新键，undefined 会导致保存时丢键
    invoke<AppSettings>("get_app_settings")
      .then((res) =>
        setSt({
          ...res,
          clipAutoPaste: res.clipAutoPaste ?? true,
          stageMax: res.stageMax ?? 50,
          stagedropEnabled: res.stagedropEnabled ?? true,
          stagedropOpenPanel: res.stagedropOpenPanel ?? false,
          appearance: res.appearance ?? "dark",
          panelTransparency: res.panelTransparency ?? true,
          superpanelHotkey: res.superpanelHotkey ?? "",
          stageHotkey: res.stageHotkey ?? "",
          vaultHotkey: res.vaultHotkey ?? "",
          clipHotkey: res.clipHotkey ?? "",
          plugins: (res.plugins ?? []).map((p) => ({
            ...p,
            entries: p.entries ?? [],
            conflicts: p.conflicts ?? [],
          })),
        })
      )
      .catch(() => {});
    // 搜索捷径清单（独立命令：与主设置分开读写）
    invoke<Shortcut[]>("shortcuts_list")
      .then(setScList)
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

  // ---------- 热键录制（原 App.tsx 机制迁入，槽位泛化到呼出+直达三处） ----------
  // 组合由 Rust 低级键盘钩子经 zap://hk-capture 事件送达——连被输入法接管的
  // Ctrl+Space（中英切换）也能捕获；页面只负责 Esc 取消与钩子启停
  const startRecording = (slot: HkSlot) => {
    invoke("start_hotkey_capture")
      .then(() => setHkSlot(slot))
      .catch((e) => setHkError("启动录制失败：" + e));
  };

  useEffect(() => {
    const un = listen<string>("zap://hk-capture", (e) => {
      if (!hkSlot) return;
      const key = e.payload;
      setSt((s) => {
        if (!s) return s;
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
  }, [hkSlot, stopHkCapture]);

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
      setSt((s) => {
        if (!s) return s;
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
  }, [hkSlot, stopHkCapture]);

  // 窗口捕获兜 Esc 关设置页（焦点掉 body 后面板 onKeyDown 收不到）。
  // 录制态让位（enabled=false，录制监听自己管 Esc=取消录制）；备份弹窗存活时
  // Esc 只关弹窗（重置口令与预览状态），不连设置页一起退。
  useEscCapture(
    () => {
      if (bkDialog) {
        setBkDialog(null);
        setBkPass("");
        setBkPass2("");
        setBkErr("");
        setBkPreview(null);
        setBkSkip(true);
        return;
      }
      onClose();
    },
    hkSlot === null
  );

  // ---------- 搜索捷径（增删改排序，本地编辑、随「保存」整单提交） ----------
  const updateSc = (i: number, patch: Partial<Shortcut>) =>
    setScList((list) => list.map((s, j) => (j === i ? { ...s, ...patch } : s)));
  const moveSc = (i: number, dir: -1 | 1) =>
    setScList((list) => {
      const j = i + dir;
      if (j < 0 || j >= list.length) return list;
      const next = [...list];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
  const delSc = (i: number) => setScList((list) => list.filter((_, j) => j !== i));
  const addSc = () =>
    setScList((list) => [...list, { keyword: "", name: "", url: "" }]);

  const saveSettings = async () => {
    if (!st) return;
    setScErr("");
    // 捷径整单先行：校验失败（空字段/重复关键词）留在本页修正，不半存
    try {
      await invoke("shortcuts_save", { list: scList });
    } catch (e) {
      setScErr(String(e));
      return;
    }
    // 开机自启随保存生效（注册表写；失败提示但不阻塞其余设置）
    try {
      await invoke("autostart_set", { enable: st.autostart });
    } catch (e) {
      toast.error("开机自启设置失败：" + String(e));
    }
    try {
      const res = await invoke<{
        hotkeyError: string | null;
        directHotkeyError: string | null;
      }>("save_settings", {
        hotkey: st.hotkey,
        vaultHotkey: st.vaultHotkey ?? "",
        clipHotkey: st.clipHotkey ?? "",
        superpanelHotkey: st.superpanelHotkey ?? "",
        stageHotkey: st.stageHotkey ?? "",
        clipEnabled: st.clipEnabled,
        clipMax: st.clipMax,
        clipAutoPaste: st.clipAutoPaste ?? true,
        stageMax: st.stageMax,
        stagedropEnabled: st.stagedropEnabled ?? true,
        stagedropOpenPanel: st.stagedropOpenPanel ?? false,
        pwdClearSecs: st.pwdClearSecs,
        everythingAutostart: st.everythingAutostart,
        appearance: st.appearance ?? "dark",
        plugins: st.plugins.map((p) => ({ id: p.id, enabled: p.enabled })),
      });
      const directError = res.directHotkeyError ?? "";
      setDirectErr(directError);
      if (res.hotkeyError || directError) {
        // 任一热键失败都重拉实际生效值：直达槽位冲突/占用时存档串与显示值会漂移；
        // 且此场景不算保存成功，不显示「已保存 ✓」
        if (res.hotkeyError) setHkError(res.hotkeyError);
        setSt(await invoke<AppSettings>("get_app_settings"));
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
      <div className="zap-panel outline-none" tabIndex={-1} ref={focusRef} />
    );
  }

  return (
    <div
      className="zap-panel vault zap-view-in outline-none"
      tabIndex={-1}
      ref={focusRef}
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
      <div className="flex-1 overflow-y-auto px-[22px] pb-3 pt-2.5">
        <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
          <Zap className="size-3 text-primary/75" />
          启动
        </div>
        <div className="flex min-h-[42px] items-center justify-between gap-3">
          <span className="text-[13px]">呼出热键</span>
          <HotkeyButton
            value={st.hotkey}
            recording={hkSlot === "main"}
            onRecord={() => startRecording("main")}
          />
        </div>
        <div className="flex min-h-[42px] items-center justify-between gap-3">
          <span className="flex min-w-0 flex-col gap-px">
            <span className="text-[13px]">账号库直达</span>
            <span className="text-[11px] leading-snug text-muted-foreground">
              呼出面板并进入账号库
            </span>
          </span>
          <HotkeyButton
            value={st.vaultHotkey ?? ""}
            recording={hkSlot === "vault"}
            onRecord={() => startRecording("vault")}
            onClear={() =>
              setSt((s) => (s ? { ...s, vaultHotkey: "" } : s))
            }
          />
        </div>
        <div className="flex min-h-[42px] items-center justify-between gap-3">
          <span className="flex min-w-0 flex-col gap-px">
            <span className="text-[13px]">快速粘贴</span>
            <span className="text-[11px] leading-snug text-muted-foreground">
              最近一条剪贴板直接粘贴到当前窗口（不经面板）
            </span>
          </span>
          <HotkeyButton
            value={st.clipHotkey ?? ""}
            recording={hkSlot === "clip"}
            onRecord={() => startRecording("clip")}
            onClear={() => setSt((s) => (s ? { ...s, clipHotkey: "" } : s))}
          />
        </div>
        {directErr && (
          <div className="pb-1 text-[11px] text-destructive">{directErr}</div>
        )}
        <div className="flex min-h-[42px] items-center justify-between gap-3">
          <span className="flex min-w-0 flex-col gap-px">
            <span className="text-[13px]">超级面板</span>
            <span className="text-[11px] leading-snug text-muted-foreground">
              按热键抓取当前选中的文字，光标旁弹出换算/复制/搜索菜单
            </span>
          </span>
          <HotkeyButton
            value={st.superpanelHotkey ?? ""}
            recording={hkSlot === "superpanel"}
            onRecord={() => startRecording("superpanel")}
            onClear={() =>
              setSt((s) => (s ? { ...s, superpanelHotkey: "" } : s))
            }
          />
        </div>
        <div className="flex min-h-[42px] items-center justify-between gap-3">
          <span className="flex min-w-0 flex-col gap-px">
            <span className="text-[13px]">中转站直达</span>
            <span className="text-[11px] leading-snug text-muted-foreground">
              呼出面板并进入中转站，可直接拖入文件/文件夹
            </span>
          </span>
          <HotkeyButton
            value={st.stageHotkey ?? ""}
            recording={hkSlot === "stage"}
            onRecord={() => startRecording("stage")}
            onClear={() => setSt((s) => (s ? { ...s, stageHotkey: "" } : s))}
          />
        </div>
        {hkError && (
          <div className="animate-shake pb-1 text-[11px] text-destructive">
            {hkError}
          </div>
        )}
        <div className="flex min-h-[42px] items-center justify-between gap-3">
          <Label
            htmlFor="set-autostart"
            className="text-[13px] font-normal text-foreground"
          >
            开机自启
          </Label>
          <Checkbox
            id="set-autostart"
            checked={st.autostart}
            onCheckedChange={(v) =>
              setSt((s) => (s ? { ...s, autostart: v === true } : s))
            }
          />
        </div>
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Clipboard className="size-3 text-primary/75" />
            剪贴板
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
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
          <div className="flex min-h-[42px] items-center justify-between gap-3">
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
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <Label
                htmlFor="set-autopaste"
                className="text-[13px] font-normal text-foreground"
              >
                选中后自动粘贴
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                历史条目回填后焦点交还原窗口并发送 Ctrl+V
              </span>
            </span>
            <Checkbox
              id="set-autopaste"
              checked={st.clipAutoPaste}
              onCheckedChange={(v) =>
                setSt((s) => (s ? { ...s, clipAutoPaste: v === true } : s))
              }
            />
          </div>
        </div>
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Inbox className="size-3 text-primary/75" />
            中转站
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <Label className="text-[13px] font-normal text-foreground">
                未固定条目容量
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                超出挤掉最旧的未固定条目；固定条目不占容量、永不清出
              </span>
            </span>
            <Select
              value={String(st.stageMax)}
              onValueChange={(v) =>
                setSt((s) => (s ? { ...s, stageMax: Number(v) } : s))
              }
            >
              <SelectTrigger className="h-[30px] w-[126px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="30">30</SelectItem>
                <SelectItem value="50">50</SelectItem>
                <SelectItem value="100">100</SelectItem>
                <SelectItem value="200">200</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <Label
                htmlFor="set-stagedrop"
                className="text-[13px] font-normal text-foreground"
              >
                拖拽落区条
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                任何应用开始拖文件时，桌面右下角弹出「拖到这，放入中转站」；部分应用不弹属正常
              </span>
            </span>
            <Checkbox
              id="set-stagedrop"
              checked={st.stagedropEnabled}
              onCheckedChange={(v) =>
                setSt((s) => (s ? { ...s, stagedropEnabled: v === true } : s))
              }
            />
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <Label
                htmlFor="set-stagedrop-open"
                className="text-[13px] font-normal text-foreground"
              >
                放入后自动打开面板
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                拖文件到落区条松手后自动呼出中转站；关闭时结果仅在落区条上短暂显示
              </span>
            </span>
            <Checkbox
              id="set-stagedrop-open"
              checked={st.stagedropOpenPanel}
              onCheckedChange={(v) =>
                setSt((s) => (s ? { ...s, stagedropOpenPanel: v === true } : s))
              }
            />
          </div>
        </div>
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <KeyRound className="size-3 text-primary/75" />
            密码安全
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
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
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Palette className="size-3 text-primary/75" />
            外观
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <Label className="text-[13px] font-normal text-foreground">
                主题
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                跟随系统实时响应系统深浅切换
              </span>
            </span>
            <Select
              value={st.appearance ?? "dark"}
              onValueChange={(v) => {
                setSt((s) => (s ? { ...s, appearance: v } : s));
                // 即时预览（保存后经 get_app_settings 同步回 AppView）
                applyAppearance(v as Appearance);
              }}
            >
              <SelectTrigger className="h-[30px] w-[126px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="dark">深色</SelectItem>
                <SelectItem value="light">浅色</SelectItem>
                <SelectItem value="system">跟随系统</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <Label
                htmlFor="set-transparency"
                className="text-[13px] font-normal text-foreground"
              >
                透明效果
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                亚克力质感，桌面透过面板隐约可见；系统不支持时保持不透明
              </span>
            </span>
            <Checkbox
              id="set-transparency"
              checked={st.panelTransparency}
              onCheckedChange={async (v) => {
                const on = v === true;
                try {
                  // 即时应用并持久化：失败（系统不支持）不落偏好、控件回弹
                  await invoke("transparency_set", { enable: on });
                  setSt((s) => (s ? { ...s, panelTransparency: on } : s));
                  applyPanelTransparency(on);
                } catch (e) {
                  toast.error("当前系统不支持透明效果：" + String(e));
                }
              }}
            />
          </div>
        </div>
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Database className="size-3 text-primary/75" />
            账号库
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <span className="text-[13px] text-foreground">备份与恢复</span>
              <span className="text-[11px] leading-snug text-muted-foreground">
                口令加密导出全部账号（含置顶），可跨机器恢复
              </span>
            </span>
            <span className="flex shrink-0 items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setBkDialog({ kind: "export" })}
              >
                导出备份
              </Button>
              <Button variant="outline" size="sm" onClick={startImport}>
                导入恢复
              </Button>
            </span>
          </div>
        </div>
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Archive className="size-3 text-primary/75" />
            配置文件
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <span className="text-[13px] text-foreground">导出与导入</span>
              <span className="text-[11px] leading-snug text-muted-foreground">
                打包全部设置、插件开关与搜索捷径；账号库走上方「备份与恢复」
              </span>
            </span>
            <span className="flex shrink-0 items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={cfgBusy}
                onClick={() => void doConfigExport()}
              >
                导出配置
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={cfgBusy}
                onClick={() => void doConfigImport()}
              >
                导入配置
              </Button>
            </span>
          </div>
        </div>
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Search className="size-3 text-primary/75" />
            Everything
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
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
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Globe className="size-3 text-primary/75" />
            搜索捷径
          </div>
          <div className="pb-1.5 text-[11px] leading-snug text-muted-foreground">
            主面板输入「关键词 + 空格 + 内容」直达搜索，URL 里用
            <code className="mx-1 rounded bg-muted px-1 font-mono">{"{query}"}</code>
            代表内容，如 gh rust clap
          </div>
          {scList.map((s, i) => (
            <div key={i} className="flex min-h-[42px] items-center gap-1.5">
              <Input
                className="h-[30px] w-[64px] shrink-0 px-2 font-mono text-xs"
                placeholder="关键词"
                spellCheck={false}
                value={s.keyword}
                onChange={(e) => updateSc(i, { keyword: e.target.value })}
              />
              <Input
                className="h-[30px] w-[88px] shrink-0 px-2"
                placeholder="名称"
                value={s.name}
                onChange={(e) => updateSc(i, { name: e.target.value })}
              />
              <Input
                className="h-[30px] min-w-0 flex-1 px-2 font-mono text-xs"
                placeholder="https://…?q={query}"
                spellCheck={false}
                value={s.url}
                onChange={(e) => updateSc(i, { url: e.target.value })}
              />
              <span className="flex shrink-0 items-center">
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 text-muted-foreground"
                  aria-label="上移"
                  disabled={i === 0}
                  onClick={() => moveSc(i, -1)}
                >
                  <ChevronUp className="size-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 text-muted-foreground"
                  aria-label="下移"
                  disabled={i === scList.length - 1}
                  onClick={() => moveSc(i, 1)}
                >
                  <ChevronDown className="size-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 text-muted-foreground hover:text-destructive"
                  aria-label="删除"
                  onClick={() => delSc(i)}
                >
                  <X className="size-3.5" />
                </Button>
              </span>
            </div>
          ))}
          <div className="flex min-h-[34px] items-center justify-between gap-3">
            <Button variant="outline" size="sm" onClick={addSc}>
              <Plus className="size-3.5" />
              添加捷径
            </Button>
            {scErr && (
              <span className="animate-shake text-[11px] text-destructive">
                {scErr}
              </span>
            )}
          </div>
        </div>
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
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
                className="flex min-h-[42px] items-center justify-between gap-3"
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
                  {(p.entries?.length ?? 0) > 0 && (
                    <span className="flex flex-wrap items-center gap-1 pt-0.5">
                      <span className="text-[11px] text-muted-foreground">
                        入口
                      </span>
                      {(p.entries ?? []).map((w) => (
                        <Kbd key={w}>{w}</Kbd>
                      ))}
                    </span>
                  )}
                  {(p.conflicts?.length ?? 0) > 0 && (
                    <span className="pt-0.5 text-[11px] leading-snug text-destructive">
                      入口词冲突：{(p.conflicts ?? []).join("、")}
                      {" · 与其他插件或搜索捷径重合，搜索时会同时出多条目"}
                    </span>
                  )}
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
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <RefreshCw className="size-3 text-primary/75" />
            软件更新
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
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
      {bkDialog && (
        <div className="zap-dialog-backdrop" onClick={closeBkDialog}>
          <div
            className="zap-dialog zap-bk-dialog"
            role="dialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
          >
            {bkDialog.kind === "export" ? (
              <>
                <div className="zap-bk-title">导出备份</div>
                <div className="zap-bk-desc">
                  全部账号将加密导出为一个 .zapvault 文件（PBKDF2 +
                  AES-256-GCM）。口令自行妥善保管，恢复时必须一致。
                </div>
                <Input
                  type="password"
                  autoFocus
                  placeholder="备份口令"
                  value={bkPass}
                  onChange={(e) => setBkPass(e.target.value)}
                />
                <Input
                  type="password"
                  placeholder="再次输入口令"
                  value={bkPass2}
                  onChange={(e) => setBkPass2(e.target.value)}
                  onKeyDown={(e) => {
                    if (
                      e.key === "Enter" &&
                      bkPass &&
                      bkPass === bkPass2 &&
                      !bkBusy
                    ) {
                      e.preventDefault();
                      void doExport();
                    }
                  }}
                />
                {bkPass2 && bkPass !== bkPass2 && (
                  <div className="zap-bk-err">两次输入的口令不一致</div>
                )}
                {bkErr && <div className="zap-bk-err">{bkErr}</div>}
                <div className="zap-confirm-actions">
                  <Button variant="outline" onClick={closeBkDialog}>
                    取消
                  </Button>
                  <Button
                    disabled={bkBusy || !bkPass || bkPass !== bkPass2}
                    onClick={() => void doExport()}
                  >
                    {bkBusy ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : null}
                    选择位置并导出
                  </Button>
                </div>
              </>
            ) : bkDialog.stage === "pass" ? (
              <>
                <div className="zap-bk-title">导入恢复</div>
                <div className="zap-bk-desc truncate" title={bkDialog.path}>
                  备份文件：{bkDialog.path}
                </div>
                <Input
                  type="password"
                  autoFocus
                  placeholder="备份口令"
                  value={bkPass}
                  onChange={(e) => setBkPass(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && bkPass && !bkBusy) {
                      e.preventDefault();
                      void doDecrypt();
                    }
                  }}
                />
                {bkErr && <div className="zap-bk-err">{bkErr}</div>}
                <div className="zap-confirm-actions">
                  <Button variant="outline" onClick={closeBkDialog}>
                    取消
                  </Button>
                  <Button
                    disabled={bkBusy || !bkPass}
                    onClick={() => void doDecrypt()}
                  >
                    {bkBusy ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : null}
                    解密预览
                  </Button>
                </div>
              </>
            ) : (
              (() => {
                const dupCount = bkPreview
                  ? bkPreview.duplicates.filter(Boolean).length
                  : 0;
                const toWrite = bkPreview
                  ? bkPreview.entries.length - (bkSkip ? dupCount : 0)
                  : 0;
                return (
                  <>
                    <div className="zap-bk-title">确认恢复</div>
                    <div className="zap-bk-desc">
                      解密成功：{bkPreview?.entries.length ?? 0} 条账号
                      {dupCount > 0 &&
                        `，其中 ${dupCount} 条与现有账号疑似重复（同标题 + 用户名）`}
                      。
                    </div>
                    <div className="zap-bk-list">
                      {bkPreview?.entries.map((e, i) => (
                        <div className="zap-bk-row" key={i}>
                          <span className="zap-bk-row-text">
                            <span className="zap-bk-row-title">
                              {e.title || e.username || "未命名账号"}
                            </span>
                            {e.username && (
                              <span className="zap-bk-row-user">
                                {e.username}
                              </span>
                            )}
                          </span>
                          {bkPreview.duplicates[i] && (
                            <span className="zap-bk-badge">重复</span>
                          )}
                        </div>
                      ))}
                    </div>
                    {dupCount > 0 && (
                      <label className="zap-bk-skip">
                        <Checkbox
                          checked={bkSkip}
                          onCheckedChange={(v) => setBkSkip(v === true)}
                        />
                        <span>跳过疑似重复的条目（推荐）</span>
                      </label>
                    )}
                    {bkErr && <div className="zap-bk-err">{bkErr}</div>}
                    {toWrite === 0 && dupCount > 0 && bkSkip && (
                      <div className="zap-bk-err">
                        全部条目均与现有账号疑似重复，无可恢复项
                      </div>
                    )}
                    <div className="zap-confirm-actions">
                      <Button variant="outline" onClick={closeBkDialog}>
                        取消
                      </Button>
                      <Button
                        disabled={bkBusy || toWrite === 0}
                        onClick={() => void doRestore()}
                      >
                        {bkBusy ? (
                          <Loader2 className="size-3.5 animate-spin" />
                        ) : null}
                        {bkSkip && dupCount > 0
                          ? `确认恢复 ${toWrite} 条`
                          : "确认恢复"}
                      </Button>
                    </div>
                  </>
                );
              })()
            )}
          </div>
        </div>
      )}
    </div>
  );
}
