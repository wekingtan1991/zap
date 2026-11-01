/// 设置页「软件更新」分区（自 SettingsView 拆出，审计 E3）：版本行 + 更新
/// 状态机展示 + 动作按钮。状态来自 AppView 挂载的 useUpdater（本组件只消费）
import { relaunch } from "@tauri-apps/plugin-process";
import { Download, Loader2, RefreshCw, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Section } from "./Section";
import type { Updater } from "../useUpdater";

interface Props {
  updater: Updater;
  appVersion: string;
}

export function UpdateSection({ updater, appVersion }: Props) {
  const {
    status: updateStatus,
    version: updateVersion,
    progress: updateProgress,
    msg: updateMsg,
  } = updater;

  return (
    <Section icon={RefreshCw} title="软件更新">
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
    </Section>
  );
}
