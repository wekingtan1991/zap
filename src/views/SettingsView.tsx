/// 设置页（从 App.tsx 迁出的自包含视图）：设置加载/保存、热键录制、
/// 各分区渲染。更新区消费 AppView 传入的 useUpdater 状态机。
/// 挂载即拉取设置，卸载即停热键钩子——开关状态由父级 view 状态机决定
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { open, save } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";
import {
  Archive,
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  Clipboard,
  Download,
  Globe,
  Inbox,
  Info,
  KeyRound,
  Loader2,
  Palette,
  Plus,
  Puzzle,
  RefreshCw,
  RotateCw,
  Search,
  Settings as SettingsIcon,
  TriangleAlert,
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
import {
  HotkeyButton,
  useHotkeyRecording,
  type HotkeySettings,
} from "./settings/useHotkeyRecording";
import { VaultBackupSection } from "./settings/VaultBackupSection";

interface PluginState {
  id: string;
  title: string;
  hint: string;
  enabled: boolean;
  /// 搜索入口词（后端 PluginMeta.entries 声明化下发）
  entries: string[];
  /// 入口词冲突（与其他插件或搜索捷径关键词重合，后端现算下发）
  conflicts: string[];
}

interface AppSettings {
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
  /// 落区条停靠角（bottom_right/bottom_left/top_right/top_left）
  stagedropPosition: string;
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

/// 搜索捷径（后端 shortcuts::Shortcut 的线协议镜像）：
/// keyword 触发词 / name 展示名 / url 含 {query} 占位的模板
interface Shortcut {
  keyword: string;
  name: string;
  url: string;
}

interface Props {
  updater: Updater;
  focusRef: (el: HTMLDivElement | null) => void;
  onClose: () => void;
}

export function SettingsView({ updater, focusRef, onClose }: Props) {
  const [st, setSt] = useState<AppSettings | null>(null);
  const [appVersion, setAppVersion] = useState("");
  /// 直达热键保存错误（与主热键错误分开呈现）
  const [directErr, setDirectErr] = useState("");
  const [saveMsg, setSaveMsg] = useState("");
  /// 搜索捷径清单：随「保存」整单提交（校验失败留页修正）
  const [scList, setScList] = useState<Shortcut[]>([]);
  const [scErr, setScErr] = useState("");
  // ---------- 脏态保护（评审 P1-03） ----------
  // 基线快照（挂载到齐 / 保存成功后写）：设置 + 捷径整体 JSON 化对比——
  // 字段改回原值仍算脏是可接受的保守语义，换取零侵入（不包 20+ 个 setSt）
  const baselineRef = useRef<string | null>(null);
  /// Esc 触发的退出确认条（有未保存更改时拦截，防静默丢弃）
  const [exitConfirm, setExitConfirm] = useState(false);
  const dirty =
    !!st &&
    baselineRef.current !== null &&
    JSON.stringify({ st, scList }) !== baselineRef.current;

  // ---------- 账号库备份：分区+弹窗+流程内聚在 VaultBackupSection（审计 E1） ----------
  // bkOpen 镜像弹窗开合：备份弹窗存活时本页 Esc 让位（弹窗组件自管关弹窗，
  // useEscCapture 互斥 enabled 口径，无顺序耦合）
  const [bkOpen, setBkOpen] = useState(false);
  /// Everything 服务状态（评审 P2-03）：文件搜索静默死亡时的自助排障入口
  const [evStatus, setEvStatus] = useState("loading");
  const [evBusy, setEvBusy] = useState(false);

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
    // 设置与捷径清单并行拉取，双双到齐后记脏判定基线（评审 P1-03）：
    // 归一化补全半新半旧会话缺失的新键（undefined 会导致保存时丢键）
    void Promise.all([
      invoke<AppSettings>("get_app_settings"),
      invoke<Shortcut[]>("shortcuts_list"),
    ])
      .then(([res, sc]) => {
        const normalized: AppSettings = {
          ...res,
          clipAutoPaste: res.clipAutoPaste ?? true,
          stageMax: res.stageMax ?? 50,
          stagedropEnabled: res.stagedropEnabled ?? true,
          stagedropOpenPanel: res.stagedropOpenPanel ?? false,
          stagedropPosition: res.stagedropPosition ?? "bottom_right",
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
        };
        setSt(normalized);
        setScList(sc);
        baselineRef.current = JSON.stringify({ st: normalized, sc });
      })
      .catch(() => {});
    // Everything 状态独立拉取（探测有网络 IO，不阻塞设置主体）
    invoke<string>("everything_status")
      .then(setEvStatus)
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

  // ---------- 热键录制（hook 化，审计 E2）：状态/监听/键帽内聚 settings/useHotkeyRecording ----------
  const patchHotkey = useCallback(
    (fn: (s: HotkeySettings) => HotkeySettings) =>
      setSt((s) => (s ? { ...s, ...fn(s) } : s)),
    [setSt]
  );
  const { hkSlot, hkError, startRecording, setHkError } =
    useHotkeyRecording(patchHotkey);

  // 窗口捕获兜 Esc 关设置页（焦点掉 body 后面板 onKeyDown 收不到）。
  // 录制态让位（enabled=false，录制监听自己管 Esc=取消录制）；备份弹窗存活时
  // 同样让位（bkOpen 由 VaultBackupSection 上报，弹窗组件自管关弹窗）。
  // 脏态拦截（评审 P1-03）：有未保存更改先弹确认条——再按 Esc=留在页面，
  // 「放弃更改」才真正退出；无更改直接退出零打扰
  useEscCapture(
    () => {
      if (exitConfirm) {
        setExitConfirm(false);
        return;
      }
      if (dirty) {
        setExitConfirm(true);
        return;
      }
      onClose();
    },
    hkSlot === null && !bkOpen
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
        stagedropPosition: st.stagedropPosition ?? "bottom_right",
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
        const fresh = await invoke<AppSettings>("get_app_settings");
        setSt(fresh);
        // 保存成功即对齐基线（权威值来自后端重拉）：dirty 归零
        baselineRef.current = JSON.stringify({ st: fresh, scList });
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
              绑定热键后，选中文字按键即抓取，光标旁弹出换算/复制/搜索菜单（默认未绑定）
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
            <span className="flex min-w-0 flex-col gap-px">
              <span className="text-[13px] text-foreground">清空未固定历史</span>
              <span className="text-[11px] leading-snug text-muted-foreground">
                立即删除全部未固定的文本与图片条目（手动复制的敏感内容补救）
              </span>
            </span>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                void invoke<number>("clip_clear_unpinned")
                  .then((n) =>
                    toast.success(n > 0 ? `已清空 ${n} 条` : "没有可清空的条目")
                  )
                  .catch((e) => toast.error(String(e)));
              }}
            >
              清空
            </Button>
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
                任何应用开始拖文件时，在设定的屏幕角落弹出「拖到这，放入中转站」；部分应用不弹属正常
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
                htmlFor="set-stagedrop-pos"
                className="text-[13px] font-normal text-foreground"
              >
                落区条位置
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                落区条停靠在拖拽发生时所在屏幕的哪个角
              </span>
            </span>
            <Select
              value={st.stagedropPosition ?? "bottom_right"}
              onValueChange={(v) =>
                setSt((s) => (s ? { ...s, stagedropPosition: v } : s))
              }
            >
              <SelectTrigger id="set-stagedrop-pos" className="h-[30px] w-[126px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="bottom_right">右下角</SelectItem>
                <SelectItem value="bottom_left">左下角</SelectItem>
                <SelectItem value="top_right">右上角</SelectItem>
                <SelectItem value="top_left">左上角</SelectItem>
              </SelectContent>
            </Select>
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
        <VaultBackupSection onDialogOpenChange={setBkOpen} />
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
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <span className="text-[13px] text-foreground">文件搜索状态</span>
              <span className="text-[11px] leading-snug text-muted-foreground">
                全盘文件搜索依赖 Everything 的 HTTP 服务（评审 P2-03）
              </span>
            </span>
            <span className="flex shrink-0 items-center gap-2">
              <span
                className={
                  "text-[11px] " +
                  (evStatus === "running" ? "text-success" : "text-muted-foreground")
                }
              >
                {evStatus === "loading"
                  ? "检测中…"
                  : evStatus === "running"
                    ? "运行中 ✓"
                    : evStatus === "installed_not_running"
                      ? "已安装未运行"
                      : evStatus === "backoff"
                        ? "启动重试中（5 分钟后）"
                        : "未安装"}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={evBusy}
                onClick={() => {
                  // 只探测不启动（ensure 带 UAC 弹窗与 5 分钟退避，不属于
                  // 「检测」语义；未运行时走呼出时的自动链路或手动安装）
                  setEvBusy(true);
                  void invoke<string>("everything_status")
                    .then(setEvStatus)
                    .catch((e) => toast.error(String(e)))
                    .finally(() => setEvBusy(false));
                }}
              >
                重新检测
              </Button>
            </span>
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
                    安装即将自动完成，zap 稍后自行重启（按钮供手动触发）
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
                重启 zap
              </Button>
            )}
          </div>
        </div>
        {/* 关于（评审 P1-16）：版本/数据位置/诊断三要素的家——SECURITY.md 的
            「设置→关于」引用在此落地，bug 反馈的日志入口（与「软件更新」区的
            版本号互补：这里是数据与诊断，不重复展示版本行） */}
        <div className="mt-3 border-t border-border pt-3">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
            <Info className="size-3 text-primary/75" />
            关于
          </div>
          <div className="flex min-h-[42px] items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col gap-px">
              <span className="text-[13px] text-foreground">数据与诊断</span>
              <span className="text-[11px] leading-snug text-muted-foreground">
                全部数据在本地目录；反馈问题请附其中的 zap.log
              </span>
            </span>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                void invoke("open_data_dir")
                  .then(() => toast.success("已打开数据目录"))
                  .catch((e) => toast.error(String(e)));
              }}
            >
              打开数据目录
            </Button>
          </div>
        </div>
      </div>
      <div className="flex shrink-0 items-center justify-end gap-2 border-t border-border px-[22px] py-1.5">
        {saveMsg && (
          <span className="shrink-0 animate-fade-in text-xs text-success">
            {saveMsg}
          </span>
        )}
        <Button size="sm" className="shrink-0" disabled={!dirty} onClick={() => void saveSettings()}>
          保存
        </Button>
      </div>
      {/* 退出确认条（评审 P1-03）：有未保存更改时 Esc/关闭被拦下——
          放弃更改走 onClose（外观预览由 App 从 settings 退出的重拉自动回滚），
          再按 Esc = 留在页面。样式复用 vault 删除确认同款 zap-dialog */}
      {exitConfirm && (
        <div className="zap-dialog-backdrop" onClick={() => setExitConfirm(false)}>
          <div
            className="zap-dialog"
            role="alertdialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="zap-confirm-text">
              <TriangleAlert className="size-3.5" />
              有未保存的更改，确定放弃并退出？
            </div>
            <div className="zap-confirm-actions">
              <Button autoFocus variant="outline" onClick={() => setExitConfirm(false)}>
                留在页面
              </Button>
              <Button variant="destructive" onClick={onClose}>
                放弃更改
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
