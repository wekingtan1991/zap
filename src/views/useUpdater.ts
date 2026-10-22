/// 自动更新状态机（从 App.tsx 迁出）：启动静默检查 + 设置页「软件更新」区共用。
/// hook 挂在 AppView（应用全生命周期，静默检查不依赖设置页是否打开），
/// SettingsView 仅消费其状态与动作
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { toast } from "sonner";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready";

export interface Updater {
  status: UpdateStatus;
  version: string;
  progress: number;
  msg: { kind: "ok" | "err"; text: string } | null;
  checkForUpdate: () => Promise<void>;
  downloadUpdate: () => Promise<void>;
}

export function useUpdater(): Updater {
  const updateRef = useRef<Update | null>(null);
  const updateNotifiedRef = useRef(false);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [updateVersion, setUpdateVersion] = useState("");
  const [updateProgress, setUpdateProgress] = useState(0);
  const [updateMsg, setUpdateMsg] = useState<
    { kind: "ok" | "err"; text: string } | null
  >(null);

  // 拿到检查结果后的公共收尾：有新版本则记录并置为可下载，否则回到空闲
  const applyUpdate = useCallback((u: Update | null) => {
    updateRef.current = u;
    if (u) {
      setUpdateVersion(u.version);
      setUpdateStatus("available");
    } else {
      setUpdateStatus("idle");
    }
  }, []);

  // 启动静默检查一次：离线/网络失败不打扰；面板隐藏时 toast 不可见，
  // 命中新版本只记标记，等下次面板呼出再提示
  useEffect(() => {
    check()
      .then((u) => {
        if (u) {
          applyUpdate(u);
          updateNotifiedRef.current = true;
        }
      })
      .catch(() => {});
  }, [applyUpdate]);

  useEffect(() => {
    const un = listen("zap://shown", () => {
      if (updateNotifiedRef.current && updateRef.current) {
        updateNotifiedRef.current = false;
        toast.info(`发现新版本 v${updateRef.current.version}，可在设置中更新`);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const checkForUpdate = useCallback(async () => {
    if (updateStatus === "checking" || updateStatus === "downloading") return;
    setUpdateStatus("checking");
    setUpdateMsg(null);
    try {
      const u = await check();
      applyUpdate(u);
      if (!u) setUpdateMsg({ kind: "ok", text: "已是最新版本 ✓" });
    } catch (e) {
      setUpdateStatus("idle");
      setUpdateMsg({ kind: "err", text: "检查失败：" + String(e) });
    }
  }, [updateStatus, applyUpdate]);

  // 下载并静默安装 NSIS 安装包；期间保持面板可见（UAC/安装器抢焦点不触发失焦即隐）
  const downloadUpdate = useCallback(async () => {
    const u = updateRef.current;
    if (!u || updateStatus === "downloading") return;
    setUpdateStatus("downloading");
    setUpdateProgress(0);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    let received = 0;
    let total = 0;
    try {
      await u.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          if (total > 0)
            setUpdateProgress(
              Math.min(99, Math.round((received / total) * 100))
            );
        }
      });
      setUpdateProgress(100);
      setUpdateStatus("ready");
    } catch (e) {
      setUpdateStatus("available");
      toast.error("下载更新失败：" + String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
    }
  }, [updateStatus]);

  return {
    status: updateStatus,
    version: updateVersion,
    progress: updateProgress,
    msg: updateMsg,
    checkForUpdate,
    downloadUpdate,
  };
}
