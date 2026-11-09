import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, Network, RefreshCw, TriangleAlert } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { useDialogA11y } from "@/hooks/useDialogA11y";
import { useEscCapture } from "@/hooks/useEscCapture";
import { PanelHeader, type PanelProps } from "@/plugins/panel";
import type { PortEntry } from "./types";

interface Props extends PanelProps {
  /// 从搜索条目带来的初始端口；null = 全量列表
  initialPort: number | null;
}

/// 端口工作台：取表与结束进程都在后端（IP Helper），前端只管展示与确认。
/// 刻意不挂 usePanelPaste —— 本面板有 input，而那个 hook 的前提是「面板无
/// 可编辑元素」（panel.tsx:51-52 明写），挂上会吞掉输入框里的粘贴。
export function PortsWorkbench({ initialPort, onBack, focusRef }: Props) {
  const [text, setText] = useState(
    initialPort === null ? "" : String(initialPort),
  );
  const [debounced, setDebounced] = useState(text);
  const [onlyListen, setOnlyListen] = useState(true);
  const [rows, setRows] = useState<PortEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  const [confirm, setConfirm] = useState<PortEntry | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const reqSeq = useRef(0);

  useDialogA11y(dialogRef, confirm !== null);
  // Esc 互斥：弹窗存活时只取消弹窗，否则退回主面板（VaultDetailView.tsx:133-136）
  useEscCapture(() => setConfirm(null), confirm !== null);
  useEscCapture(onBack, confirm === null);

  // 过滤词防抖：端口走后端 filter，进程名/PID 走前端二次过滤
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(text), 150);
    return () => clearTimeout(timer);
  }, [text]);

  const filterPort = useMemo(() => {
    const t = debounced.trim();
    if (!/^\d{1,5}$/.test(t)) return null;
    const n = Number(t);
    return n > 0 && n <= 65535 ? n : null;
  }, [debounced]);

  const load = useCallback(async () => {
    const seq = ++reqSeq.current;
    setLoading(true);
    try {
      const list = await invoke<PortEntry[]>("ports_list", {
        filter: filterPort,
        onlyListen,
      });
      if (seq !== reqSeq.current) return;
      setRows(list);
      setErr("");
    } catch (e) {
      if (seq !== reqSeq.current) return;
      setErr(String(e));
      setRows([]);
    } finally {
      if (seq === reqSeq.current) setLoading(false);
    }
  }, [filterPort, onlyListen]);

  useEffect(() => {
    void load();
  }, [load]);

  const shown = useMemo(() => {
    const t = debounced.trim().toLowerCase();
    if (!t || filterPort !== null) return rows;
    return rows.filter(
      (r) =>
        r.processName.toLowerCase().includes(t) ||
        String(r.pid).includes(t) ||
        r.localAddr.toLowerCase().includes(t),
    );
  }, [rows, debounced, filterPort]);

  // 吸附态由真实滚动位置驱动，类名直接挂 DOM：滚动是高频事件，每次
  // setState 都会拖着整张表重渲。行数变化时重新同步一次，避免空态切回
  // 表格后残留旧的 is-scrolled。
  useEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const sync = () => el.classList.toggle("is-scrolled", el.scrollTop > 0);
    sync();
    el.addEventListener("scroll", sync, { passive: true });
    return () => el.removeEventListener("scroll", sync);
  }, [shown.length]);

  const doKill = useCallback(
    async (row: PortEntry) => {
      setConfirm(null);
      try {
        const msg = await invoke<string>("ports_kill", {
          port: row.port,
          pid: row.pid,
          startedAt: row.startedAt,
        });
        toast.success(msg);
      } catch (e) {
        toast.error(String(e));
      }
      await load();
    },
    [load],
  );

  const permissionDenied = /拒绝访问|access denied|Win32 5\b/i.test(err);

  return (
    <div className="zap-panel zap-ports-panel outline-none" tabIndex={-1} ref={focusRef}>
      <PanelHeader icon={<Network className="size-4" />} tag="端口查询">
        {!err && (
          <span className="zap-ports-count">
            {loading ? "查询中…" : `共 ${shown.length} 条`}
          </span>
        )}
        <input
          className="zap-ports-filter"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="端口 / 进程名 / PID"
          aria-label="过滤端口或进程"
        />
        <label className="zap-ports-toggle">
          <input
            type="checkbox"
            checked={onlyListen}
            onChange={(e) => setOnlyListen(e.target.checked)}
          />
          只看监听
        </label>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="刷新"
          onClick={() => void load()}
        >
          <RefreshCw className="size-3.5" />
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={onBack}
        >
          <ArrowLeft className="size-3" />
          返回（Esc）
        </Button>
      </PanelHeader>

      <div className="zap-ports-body" ref={bodyRef}>
        {err ? (
          <div className="zap-ports-empty">
            <TriangleAlert className="size-4" />
            <span>
              {permissionDenied
                ? "权限不足：请以管理员身份运行 zap 后重试"
                : `端口表读取失败：${err}`}
            </span>
          </div>
        ) : loading ? (
          <div className="zap-ports-empty">正在查询端口占用…</div>
        ) : shown.length === 0 ? (
          <div className="zap-ports-empty">
            {filterPort !== null ? (
              <>
                <span>
                  端口 {filterPort} 当前未被进程占用，可以直接启动服务。
                </span>
                <span className="zap-ports-hint">
                  若仍报「Address already in use」，可能被 Windows / Hyper-V / WSL
                  保留端口段占用，可用 netsh interface ipv4 show excludedportrange
                  protocol=tcp 查看。
                </span>
              </>
            ) : (
              <span>没有匹配的端口占用记录。</span>
            )}
          </div>
        ) : (
          <table className="zap-ports-table">
            <thead>
              <tr>
                <th>端口</th>
                <th>协议</th>
                <th>状态</th>
                <th>本地地址</th>
                <th>PID</th>
                <th>进程</th>
                <th aria-label="操作" />
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <tr
                  key={`${r.port}:${r.pid}`}
                  className={
                    initialPort === r.port ? "zap-ports-hit" : undefined
                  }
                >
                  <td className="zap-ports-port">{r.port}</td>
                  <td>{r.family}</td>
                  <td>{r.state}</td>
                  <td className="zap-ports-mono" title={r.localAddr}>
                    {r.localAddr}
                  </td>
                  <td>{r.pid}</td>
                  <td
                    className="zap-ports-proc"
                    title={r.processPath ?? r.processName}
                  >
                    {r.processName}
                  </td>
                  <td>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="zap-ports-kill"
                      disabled={r.protected}
                      title={
                        r.protected
                          ? "系统/受保护进程，不能结束"
                          : `结束 ${r.processName}（PID ${r.pid}）`
                      }
                      onClick={() => setConfirm(r)}
                    >
                      结束
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {confirm && (
        <div
          className="zap-dialog-backdrop"
          onClick={() => setConfirm(null)}
        >
          <div
            ref={dialogRef}
            className="zap-dialog zap-ports-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-label="结束进程确认"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="zap-confirm-text">
              <TriangleAlert className="size-3.5" />
              强制结束可能丢失未保存数据，确认结束端口&nbsp;{confirm.port} 的进程？
            </div>
            <div className="zap-ports-dialog-info">
              <div>
                <span>PID</span>
                <b>{confirm.pid}</b>
              </div>
              <div>
                <span>进程</span>
                <b>{confirm.processName}</b>
              </div>
              <div>
                <span>路径</span>
                <b title={confirm.processPath ?? ""}>
                  {confirm.processPath ?? "（未知）"}
                </b>
              </div>
              <div>
                <span>启动时间</span>
                <b>
                  {confirm.startedAt
                    ? new Date(confirm.startedAt).toLocaleString()
                    : "（未知）"}
                </b>
              </div>
            </div>
            <div className="zap-confirm-actions">
              {/* 安全动作降为 outline：危险动作才是唯一着色按钮（设置页退出
                  确认同款层级），焦点仍默认停在取消上 */}
              <Button autoFocus variant="outline" onClick={() => setConfirm(null)}>
                取消
              </Button>
              <Button
                variant="destructive"
                onClick={() => void doKill(confirm)}
              >
                确认结束
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
