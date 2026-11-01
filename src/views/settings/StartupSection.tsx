/// 设置页「启动」分区（自 SettingsView 拆出，审计 E3）：呼出热键 + 四个直达
/// 热键槽位 + 开机自启。热键录制态与错误提示由 useHotkeyRecording hook 持有、
/// 经 props 下发
import type { Dispatch, SetStateAction } from "react";
import { Zap } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Section } from "./Section";
import { HotkeyButton, type HkSlot } from "./useHotkeyRecording";
import type { AppSettings } from "./types";

interface Props {
  st: AppSettings;
  setSt: Dispatch<SetStateAction<AppSettings | null>>;
  hkSlot: HkSlot | null;
  hkError: string;
  /// 直达热键保存错误（与主热键错误分开呈现）
  directErr: string;
  startRecording: (slot: HkSlot) => void;
}

export function StartupSection({
  st,
  setSt,
  hkSlot,
  hkError,
  directErr,
  startRecording,
}: Props) {
  return (
    <Section icon={Zap} title="启动">
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
    </Section>
  );
}
