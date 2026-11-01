/// 设置页「Everything」分区（自 SettingsView 拆出，审计 E3）：自动启动开关 +
/// 服务状态行（评审 P2-03：文件搜索静默死亡时的自助排障入口）。状态拉取与
/// 重新检测自持——只探测不启动（ensure 带 UAC 弹窗与 5 分钟退避，不属于
/// 「检测」语义；未运行时走呼出时的自动链路或手动安装）
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import type { Dispatch, SetStateAction } from "react";
import { Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Section } from "./Section";
import type { AppSettings } from "./types";

interface Props {
  st: AppSettings;
  setSt: Dispatch<SetStateAction<AppSettings | null>>;
}

export function EverythingSection({ st, setSt }: Props) {
  const [evStatus, setEvStatus] = useState("loading");
  const [evBusy, setEvBusy] = useState(false);

  useEffect(() => {
    // Everything 状态独立拉取（探测有网络 IO，不阻塞设置主体）
    invoke<string>("everything_status")
      .then(setEvStatus)
      .catch(() => {});
  }, []);

  return (
    <Section icon={Search} title="Everything">
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
    </Section>
  );
}
