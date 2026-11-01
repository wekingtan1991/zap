/// 账号库备份分区 + 弹窗（自 SettingsView 拆出，审计 E1）：加密导出/恢复全流程
/// 内聚——状态、动作、分区 UI 与弹窗同文件。动作即时执行不随「保存」。
/// 导出：弹窗填口令（二次确认）→ save 对话框 → 加密落盘；
/// 导入：open 对话框选文件 → 弹窗输口令 → 解密预览 → 确认恢复（追加写入）。
/// 文件对话框会抢主窗口焦点触发「失焦即隐」：期间一律 set_hold_panel 保前台。
/// 弹窗存活时本组件自管 Esc（关弹窗并重置口令/预览），父级设置页 Esc 经
/// onDialogOpenChange 让位（useEscCapture 的互斥 enabled 口径）
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { Database, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { useEscCapture } from "@/hooks/useEscCapture";
import { backupReminder } from "@/lib/vaultBackup";

/// 备份条目（后端 backup::BackupEntry 的线协议镜像）
interface BackupEntry {
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
  pinned: number;
}

/// 解密预览（后端 backup::BackupPreview）：条目 + 疑似重复标记
interface BackupPreview {
  entries: BackupEntry[];
  duplicates: boolean[];
}

export function VaultBackupSection({
  onDialogOpenChange,
}: {
  onDialogOpenChange: (open: boolean) => void;
}) {
  const [bkDialog, setBkDialog] = useState<
    | { kind: "export" }
    | { kind: "import"; path: string; stage: "pass" | "confirm" }
    | null
  >(null);
  const [bkPass, setBkPass] = useState("");
  const [bkPass2, setBkPass2] = useState("");
  const [bkBusy, setBkBusy] = useState(false);
  const [bkErr, setBkErr] = useState("");
  const [bkPreview, setBkPreview] = useState<BackupPreview | null>(null);
  /// 恢复模式：跳过疑似重复（默认推荐）/ 全部追加
  const [bkSkip, setBkSkip] = useState(true);
  /// 上次成功导出的 Unix 毫秒（null = 尚未取到；0 = 从未备份）
  const [bkLast, setBkLast] = useState<number | null>(null);

  // 弹窗开合上报：父级设置页的 Esc 据此让位（互斥 enabled，无顺序耦合）
  useEffect(() => {
    onDialogOpenChange(bkDialog !== null);
  }, [bkDialog, onDialogOpenChange]);

  const pullBkLast = useCallback(() => {
    invoke<number>("vault_last_export_at")
      .then((v) => setBkLast(v))
      .catch(() => {}); // 提醒是辅助信息，取不到就静默
  }, []);
  useEffect(pullBkLast, [pullBkLast]);

  const closeBkDialog = () => {
    setBkDialog(null);
    setBkPass("");
    setBkPass2("");
    setBkErr("");
    setBkPreview(null);
    setBkSkip(true);
  };

  const doExport = async () => {
    setBkBusy(true);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    try {
      const path = await save({
        defaultPath: `zap-vault-${new Date().toISOString().slice(0, 10)}.zapvault`,
        filters: [{ name: "zap 账号备份", extensions: ["zapvault"] }],
      });
      if (!path) return; // 取消选位：留在弹窗继续
      const n = await invoke<number>("vault_backup_export", {
        path,
        passphrase: bkPass,
      });
      toast.success(`已导出 ${n} 条账号`);
      pullBkLast(); // 提醒行随新导出即时收敛
      closeBkDialog();
    } catch (e) {
      setBkErr(String(e));
      toast.error(String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
      setBkBusy(false);
    }
  };

  const startImport = async () => {
    setBkBusy(true);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "zap 账号备份", extensions: ["zapvault"] }],
      });
      if (path && typeof path === "string") {
        setBkDialog({ kind: "import", path, stage: "pass" });
        setBkErr("");
      }
    } catch (e) {
      toast.error(String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
      setBkBusy(false);
    }
  };

  const doDecrypt = async () => {
    if (!bkDialog || bkDialog.kind !== "import") return;
    setBkBusy(true);
    try {
      const preview = await invoke<BackupPreview>("vault_backup_decrypt", {
        path: bkDialog.path,
        passphrase: bkPass,
      });
      setBkPreview(preview);
      setBkSkip(true);
      setBkDialog({ ...bkDialog, stage: "confirm" });
      setBkErr("");
    } catch (e) {
      // 口令错误等留在弹窗内重输，不 toast（输入焦点还在原处）
      setBkErr(String(e));
    } finally {
      setBkBusy(false);
    }
  };

  const doRestore = async () => {
    if (!bkPreview) return;
    setBkBusy(true);
    try {
      const [written, skipped] = await invoke<[number, number]>(
        "vault_backup_restore",
        { entries: bkPreview.entries, skipDuplicates: bkSkip }
      );
      toast.success(
        skipped > 0
          ? `已恢复 ${written} 条账号，跳过重复 ${skipped} 条`
          : `已恢复 ${written} 条账号`
      );
      closeBkDialog();
    } catch (e) {
      setBkErr("恢复失败：" + String(e));
      toast.error("恢复失败：" + String(e));
    } finally {
      setBkBusy(false);
    }
  };

  // 弹窗存活时 Esc 无条件关弹窗（重置口令与预览），不连设置页一起退——
  // 父级 Esc 由 onDialogOpenChange 上报的布尔互斥让位
  useEscCapture(closeBkDialog, bkDialog !== null);

  return (
    <>
      <div className="mt-3 border-t border-border pt-3">
        <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
          <Database className="size-3 text-primary/75" />
          账号库
        </div>
        <div className="flex min-h-[42px] items-center justify-between gap-3">
          <span className="flex min-w-0 flex-col gap-px">
            <span className="text-[13px] text-foreground">备份与恢复</span>
            <span className="text-[11px] leading-snug text-muted-foreground">
              口令加密导出全部账号（含置顶），可跨机器恢复；数据库异常被重置时，数据可由备份找回
            </span>
          </span>
          <span className="flex shrink-0 items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setBkDialog({ kind: "export" })}
            >
              导出备份
            </Button>
            <Button variant="outline" size="sm" onClick={() => void startImport()}>
              导入恢复
            </Button>
          </span>
        </div>
        {bkLast !== null &&
          (() => {
            const r = backupReminder(bkLast, Date.now());
            const text =
              r.level === "never"
                ? "尚未备份 · 数据只在本地一份，建议立即导出"
                : r.level === "stale"
                  ? `上次备份已 ${r.days} 天 · 建议尽快导出`
                  : `上次备份：${r.days === 0 ? "今天" : `${r.days} 天前`}`;
            return (
              <div
                className={`mt-1 text-[11px] ${
                  r.level === "ok" ? "text-muted-foreground" : "text-destructive"
                }`}
              >
                {text}
              </div>
            );
          })()}
      </div>
      {bkDialog && (
        <div className="zap-dialog-backdrop" onClick={closeBkDialog}>
          <div
            className="zap-dialog zap-bk-dialog"
            role="dialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
          >
            {bkDialog.kind === "export" ? (
              <>
                <div className="zap-bk-title">导出备份</div>
                <div className="zap-bk-desc">
                  全部账号将加密导出为一个 .zapvault 文件（PBKDF2 +
                  AES-256-GCM）。口令自行妥善保管，恢复时必须一致。
                </div>
                <Input
                  type="password"
                  autoFocus
                  placeholder="备份口令"
                  value={bkPass}
                  onChange={(e) => setBkPass(e.target.value)}
                />
                <Input
                  type="password"
                  placeholder="再次输入口令"
                  value={bkPass2}
                  onChange={(e) => setBkPass2(e.target.value)}
                  onKeyDown={(e) => {
                    if (
                      e.key === "Enter" &&
                      bkPass &&
                      bkPass === bkPass2 &&
                      !bkBusy
                    ) {
                      e.preventDefault();
                      void doExport();
                    }
                  }}
                />
                {bkPass2 && bkPass !== bkPass2 && (
                  <div className="zap-bk-err">两次输入的口令不一致</div>
                )}
                {bkErr && <div className="zap-bk-err">{bkErr}</div>}
                <div className="zap-confirm-actions">
                  <Button variant="outline" onClick={closeBkDialog}>
                    取消
                  </Button>
                  <Button
                    disabled={bkBusy || !bkPass || bkPass !== bkPass2}
                    onClick={() => void doExport()}
                  >
                    {bkBusy ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : null}
                    选择位置并导出
                  </Button>
                </div>
              </>
            ) : bkDialog.stage === "pass" ? (
              <>
                <div className="zap-bk-title">导入恢复</div>
                <div className="zap-bk-desc truncate" title={bkDialog.path}>
                  备份文件：{bkDialog.path}
                </div>
                <Input
                  type="password"
                  autoFocus
                  placeholder="备份口令"
                  value={bkPass}
                  onChange={(e) => setBkPass(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && bkPass && !bkBusy) {
                      e.preventDefault();
                      void doDecrypt();
                    }
                  }}
                />
                {bkErr && <div className="zap-bk-err">{bkErr}</div>}
                <div className="zap-confirm-actions">
                  <Button variant="outline" onClick={closeBkDialog}>
                    取消
                  </Button>
                  <Button
                    disabled={bkBusy || !bkPass}
                    onClick={() => void doDecrypt()}
                  >
                    {bkBusy ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : null}
                    解密预览
                  </Button>
                </div>
              </>
            ) : (
              (() => {
                const dupCount = bkPreview
                  ? bkPreview.duplicates.filter(Boolean).length
                  : 0;
                const toWrite = bkPreview
                  ? bkPreview.entries.length - (bkSkip ? dupCount : 0)
                  : 0;
                return (
                  <>
                    <div className="zap-bk-title">确认恢复</div>
                    <div className="zap-bk-desc">
                      解密成功：{bkPreview?.entries.length ?? 0} 条账号
                      {dupCount > 0 &&
                        `，其中 ${dupCount} 条与现有账号疑似重复（同标题 + 用户名）`}
                      。
                    </div>
                    <div className="zap-bk-list">
                      {bkPreview?.entries.map((e, i) => (
                        <div className="zap-bk-row" key={i}>
                          <span className="zap-bk-row-text">
                            <span className="zap-bk-row-title">
                              {e.title || e.username || "未命名账号"}
                            </span>
                            {e.username && (
                              <span className="zap-bk-row-user">
                                {e.username}
                              </span>
                            )}
                          </span>
                          {bkPreview.duplicates[i] && (
                            <span className="zap-bk-badge">重复</span>
                          )}
                        </div>
                      ))}
                    </div>
                    {dupCount > 0 && (
                      <label className="zap-bk-skip">
                        <Checkbox
                          checked={bkSkip}
                          onCheckedChange={(v) => setBkSkip(v === true)}
                        />
                        <span>跳过疑似重复的条目（推荐）</span>
                      </label>
                    )}
                    {bkErr && <div className="zap-bk-err">{bkErr}</div>}
                    {toWrite === 0 && dupCount > 0 && bkSkip && (
                      <div className="zap-bk-err">
                        全部条目均与现有账号疑似重复，无可恢复项
                      </div>
                    )}
                    <div className="zap-confirm-actions">
                      <Button variant="outline" onClick={closeBkDialog}>
                        取消
                      </Button>
                      <Button
                        disabled={bkBusy || toWrite === 0}
                        onClick={() => void doRestore()}
                      >
                        {bkBusy ? (
                          <Loader2 className="size-3.5 animate-spin" />
                        ) : null}
                        {bkSkip && dupCount > 0
                          ? `确认恢复 ${toWrite} 条`
                          : "确认恢复"}
                      </Button>
                    </div>
                  </>
                );
              })()
            )}
          </div>
        </div>
      )}
    </>
  );
}
