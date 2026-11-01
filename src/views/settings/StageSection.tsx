/// 设置页「中转站」分区（自 SettingsView 拆出，审计 E3）：未固定容量、
/// 拖拽落区条三项（开关/停靠角/放入后自动开面板）
import type { Dispatch, SetStateAction } from "react";
import { Inbox } from "lucide-react";
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

export function StageSection({ st, setSt }: Props) {
  return (
    <Section icon={Inbox} title="中转站">
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
    </Section>
  );
}
