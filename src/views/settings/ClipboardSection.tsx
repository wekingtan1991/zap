/// 设置页「剪贴板」分区（自 SettingsView 拆出，审计 E3）：记录开关、保留
/// 条数、选中即粘贴；清空未固定历史为即时动作（不随「保存」）
import type { Dispatch, SetStateAction } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Clipboard } from "lucide-react";
import { Button } from "@/components/ui/button";
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
import type { AppSettings } from "./types";

interface Props {
  st: AppSettings;
  setSt: Dispatch<SetStateAction<AppSettings | null>>;
}

export function ClipboardSection({ st, setSt }: Props) {
  return (
    <Section icon={Clipboard} title="剪贴板">
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
    </Section>
  );
}
