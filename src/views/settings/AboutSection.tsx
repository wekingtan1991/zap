/// 设置页「关于」分区（自 SettingsView 拆出，审计 E3）：版本/数据位置/诊断
/// 三要素的家——SECURITY.md 的「设置→关于」引用在此落地，bug 反馈的日志入口
/// （与「软件更新」区的版本号互补：这里是数据与诊断，不重复展示版本行）
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Info } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Section } from "./Section";

export function AboutSection() {
  return (
    <Section icon={Info} title="关于">
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
    </Section>
  );
}
