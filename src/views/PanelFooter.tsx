/// 主面板底栏（自 App.tsx 拆出，审计 F1）：快捷键提示 + 合并模式状态/入口 +
/// Alt 组合键与次级操作悬停说明。纯展示组件——状态由 AppView 持有经 props 下发
import { CircleHelp, Layers } from "lucide-react";
import { Kbd } from "@/components/ui/kbd";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

/// 账号库保护域的 Alt 组合键说明（底栏问号图标的悬停提示内容）
const ALT_ACTIONS: [string, string][] = [
  ["1", "打开网址"],
  ["2", "复制密码（阅后即焚）"],
  ["3", "删除（弹确认条，不会直接删）"],
  ["4", "查看密码"],
  ["5", "置顶 / 取消置顶"],
  ["6", "编辑"],
];

/// 普通域条目的次级操作说明（底栏问号图标悬停提示）
const SECONDARY_ACTIONS: [string, string][] = [
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
        {isVault ? (
          <>
            <span className="zap-footer-sep" />
            <Kbd>Alt</Kbd>
            <span>+</span>
            <Kbd>1~6</Kbd>
            <span>行内操作</span>
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  className="zap-footer-help"
                  aria-label="Alt 组合键说明"
                  tabIndex={-1}
                >
                  <CircleHelp className="size-3.5" />
                </span>
              </TooltipTrigger>
              <TooltipContent side="top" align="start">
                <div className="zap-alt-help">
                  {ALT_ACTIONS.map(([key, desc]) => (
                    <div key={key} className="zap-alt-help-row">
                      <span className="zap-alt-help-keys">
                        <Kbd>Alt</Kbd>
                        <span>+</span>
                        <Kbd>{key}</Kbd>
                      </span>
                      <span>{desc}</span>
                    </div>
                  ))}
                </div>
              </TooltipContent>
            </Tooltip>
          </>
        ) : (
          <>
            <span className="zap-footer-sep" />
            {mergeMode ? (
              <>
                <span className="zap-merge-status">
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
            <span className="zap-footer-sep" />
            <Kbd>pw</Kbd>
            <span>账号库</span>
            <span className="zap-footer-sep" />
            <Kbd>Ctrl</Kbd>
            <span>+</span>
            <Kbd>V</Kbd>
            <span>粘贴即识别</span>
            <span className="zap-footer-sep" />
            <Kbd>json</Kbd>
            <span>/</span>
            <Kbd>tool</Kbd>
            <span>搜索工具</span>
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  className="zap-footer-help"
                  aria-label="次级操作说明"
                  tabIndex={-1}
                >
                  <CircleHelp className="size-3.5" />
                </span>
              </TooltipTrigger>
              <TooltipContent side="top" align="start">
                <div className="zap-alt-help">
                  {SECONDARY_ACTIONS.map(([keys, desc]) => (
                    <div key={keys} className="zap-alt-help-row">
                      <span className="zap-alt-help-keys">
                        {keys.split("+").map((k, i) => (
                          <span
                            key={k}
                            className="flex items-center gap-0.5"
                          >
                            {i > 0 && <span>+</span>}
                            <Kbd>{k}</Kbd>
                          </span>
                        ))}
                      </span>
                      <span>{desc}</span>
                    </div>
                  ))}
                </div>
              </TooltipContent>
            </Tooltip>
          </>
        )}
      </span>
      <span className="zap-footer-hints">
        <Kbd>Ctrl</Kbd>
        <span>+</span>
        <Kbd>,</Kbd>
        <span>设置</span>
        <span className="zap-footer-sep" />
        <Kbd>Esc</Kbd>
        {/* 合并模式下 Esc 的实际语义是退出合并（评审 P1-05）：提示随模式切换 */}
        <span>{mergeMode ? "退出合并" : "清空 / 关闭"}</span>
      </span>
    </div>
  );
}
