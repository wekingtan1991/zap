/// 设置页「外观」分区（自 SettingsView 拆出，审计 E3）：主题（即时预览）、
/// 透明效果（即时应用并持久化，失败回弹）
import type { Dispatch, SetStateAction } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Palette } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Section } from "./Section";
import {
  applyAppearance,
  applyPanelTransparency,
  type Appearance,
} from "@/lib/appearance";
import type { AppSettings } from "./types";

interface Props {
  st: AppSettings;
  setSt: Dispatch<SetStateAction<AppSettings | null>>;
}

export function AppearanceSection({ st, setSt }: Props) {
  return (
    <Section icon={Palette} title="外观">
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
    </Section>
  );
}
