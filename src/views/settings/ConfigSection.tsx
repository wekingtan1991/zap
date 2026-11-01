/// 设置页「配置文件」分区（自 SettingsView 拆出，审计 E3）：设置+插件开关+
/// 捷径打包导出；导入回填表单，随「保存」生效。导入动作改写父级 st 与
/// scList（经 props 桥接），busy 态自持
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import type { Dispatch, SetStateAction } from "react";
import { Archive } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Section } from "./Section";
import {
  countImportedSettings,
  mergeImportedConfig,
  type ConfigFilePayload,
} from "@/lib/configfile";
import type { AppSettings, Shortcut } from "./types";

interface Props {
  setSt: Dispatch<SetStateAction<AppSettings | null>>;
  setScList: Dispatch<SetStateAction<Shortcut[]>>;
}

export function ConfigSection({ setSt, setScList }: Props) {
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

  return (
    <Section icon={Archive} title="配置文件">
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
    </Section>
  );
}
