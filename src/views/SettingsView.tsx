/// 设置页（从 App.tsx 迁出的自包含视图；审计 E 批拆分后主干只保留状态机、
/// 加载/保存与分区装配）：设置加载/保存、脏态保护、退出确认。各分区组件与
/// 热键录制 hook、账号库备份域在 settings/ 目录——接口/保存链路变更时主干
/// 与对应分区各改一处。更新区消费 AppView 传入的 useUpdater 状态机。
/// 挂载即拉取设置，卸载即停热键钩子——开关状态由父级 view 状态机决定
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { toast } from "sonner";
import { ArrowLeft, Settings as SettingsIcon, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { Updater } from "./useUpdater";
import { useEscCapture } from "@/hooks/useEscCapture";
import { VaultBackupSection } from "./settings/VaultBackupSection";
import { StartupSection } from "./settings/StartupSection";
import { ClipboardSection } from "./settings/ClipboardSection";
import { StageSection } from "./settings/StageSection";
import { SecuritySection } from "./settings/SecuritySection";
import { AppearanceSection } from "./settings/AppearanceSection";
import { ConfigSection } from "./settings/ConfigSection";
import { EverythingSection } from "./settings/EverythingSection";
import { ShortcutsSection } from "./settings/ShortcutsSection";
import { PluginsSection } from "./settings/PluginsSection";
import { UpdateSection } from "./settings/UpdateSection";
import { AboutSection } from "./settings/AboutSection";
import {
  useHotkeyRecording,
  type HotkeySettings,
} from "./settings/useHotkeyRecording";
import type { AppSettings, Shortcut } from "./settings/types";

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
        <StartupSection
          st={st}
          setSt={setSt}
          hkSlot={hkSlot}
          hkError={hkError}
          directErr={directErr}
          startRecording={startRecording}
        />
        <ClipboardSection st={st} setSt={setSt} />
        <StageSection st={st} setSt={setSt} />
        <SecuritySection st={st} setSt={setSt} />
        <AppearanceSection st={st} setSt={setSt} />
        <VaultBackupSection onDialogOpenChange={setBkOpen} />
        <ConfigSection setSt={setSt} setScList={setScList} />
        <EverythingSection st={st} setSt={setSt} />
        <ShortcutsSection scList={scList} scErr={scErr} setScList={setScList} />
        <PluginsSection st={st} setSt={setSt} />
        <UpdateSection updater={updater} appVersion={appVersion} />
        <AboutSection />
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
