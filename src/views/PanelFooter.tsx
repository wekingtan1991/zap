/// 主面板底栏（自 App.tsx 拆出，审计 F1）：快捷键提示 + 合并模式状态/入口 +
/// 其余快捷键的「?」悬停说明。纯展示组件——状态由 AppView 持有经 props 下发
import { CircleHelp, Layers } from "lucide-react";
import { Kbd } from "@/components/ui/kbd";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

/// 「?」浮层行渲染：键位一列、说明一列（组合键按 + 拆分逐枚举）
function HelpList({ rows }: { rows: [string, string][] }) {
  return (
    <div className="zap-alt-help">
      {rows.map(([keys, desc]) => (
        <div key={keys} className="zap-alt-help-row">
          <span className="zap-alt-help-keys">
            {keys.split("+").map((k, i) => (
              <span key={k} className="flex items-center gap-0.5">
                {i > 0 && <span>+</span>}
                <Kbd>{k}</Kbd>
              </span>
            ))}
          </span>
          <span>{desc}</span>
        </div>
      ))}
    </div>
  );
}

/// 账号库保护域的 Alt 行内操作说明（底栏问号图标的悬停提示内容）
const VAULT_HELP: [string, string][] = [
  ["Alt+1", "打开网址"],
  ["Alt+2", "复制密码（延时清空）"],
  ["Alt+3", "删除（弹确认条，不会直接删）"],
  ["Alt+4", "查看密码"],
  ["Alt+5", "置顶 / 取消置顶"],
  ["Alt+6", "编辑"],
  ["Ctrl+,", "设置"],
];

/// 普通域的其余快捷键说明（底栏问号图标悬停提示）：全局入口 + 条目次级操作
const NORMAL_HELP: [string, string][] = [
  ["pw", "账号库"],
  ["Ctrl+V", "粘贴即识别"],
  ["Ctrl+,", "设置"],
  ["Ctrl+Shift+Enter", "管理员运行"],
  ["Ctrl+Enter", "打开所在文件夹"],
  ["Ctrl+Shift+C", "复制路径"],
];

interface Props {
  isVault: boolean;
  mergeMode: boolean;
  /// 合并模式已勾选条数
  mergeSelected: number;
  /// 列表实际含 clip 条目（合并入口只在此露出，评审 P1-05）
  hasClipItems: boolean;
  onMergeGo: () => void;
  onToggleMerge: () => void;
}

export function PanelFooter({
  isVault,
  mergeMode,
  mergeSelected,
  hasClipItems,
  onMergeGo,
  onToggleMerge,
}: Props) {
  return (
    <div className="zap-footer">
      <span className="zap-footer-hints">
        <Kbd>↑</Kbd>
        <Kbd>↓</Kbd>
        <span>选择</span>
        <span className="zap-footer-sep" />
        <Kbd>Enter</Kbd>
        <span>{isVault ? "详情" : "启动"}</span>
        {!isVault && (
          <>
            <span className="zap-footer-sep" />
            {mergeMode ? (
              <>
                <span className="zap-merge-status" aria-live="polite">
                  合并模式 · 已选 {mergeSelected} 条
                </span>
                <button
                  type="button"
                  className="zap-merge-go"
                  disabled={mergeSelected === 0}
                  onClick={onMergeGo}
                >
                  <Kbd>Ctrl</Kbd>
                  <span>+</span>
                  <Kbd>Enter</Kbd>
                  <span>合并粘贴</span>
                </button>
                <button
                  type="button"
                  className="zap-merge-exit"
                  onClick={onToggleMerge}
                >
                  退出
                </button>
              </>
            ) : (
              // 合并入口只在列表实际含 clip 条目时露出（评审 P1-05）：
              // 对纯应用/文件结果显示「合并粘贴」是无指向的死按钮
              hasClipItems && (
                <button
                  type="button"
                  className="zap-merge-enter"
                  onClick={onToggleMerge}
                  title="连续点选多条剪贴板文本条目，按点选顺序合并成一条粘贴"
                >
                  <Layers className="size-3" />
                  <span>合并粘贴</span>
                </button>
              )
            )}
          </>
        )}
        <span className="zap-footer-sep" />
        <Tooltip>
          <TooltipTrigger asChild>
            {/* 可聚焦按钮（R3 P2）：tabIndex=-1 的 span 让 radix Tooltip
                的 focus 触发通路永不发生，aria-label 挂 generic span 也不
                暴露——双失效改单点修复 */}
            <button
              type="button"
              className="zap-footer-help"
              aria-label="更多快捷键说明"
            >
              <CircleHelp className="size-3.5" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="top" align="start">
            <HelpList rows={isVault ? VAULT_HELP : NORMAL_HELP} />
          </TooltipContent>
        </Tooltip>
      </span>
      <span className="zap-footer-hints">
        <Kbd>Esc</Kbd>
        {/* 合并模式下 Esc 的实际语义是退出合并（评审 P1-05）：提示随模式切换 */}
        <span>{mergeMode ? "退出合并" : "清空 / 关闭"}</span>
      </span>
    </div>
  );
}
