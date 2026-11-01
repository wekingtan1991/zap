/// 设置页「密码安全」分区（自 SettingsView 拆出，审计 E3）：复制后焚除延时
import type { Dispatch, SetStateAction } from "react";
import { KeyRound } from "lucide-react";
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

export function SecuritySection({ st, setSt }: Props) {
  return (
    <Section icon={KeyRound} title="密码安全">
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
    </Section>
  );
}
