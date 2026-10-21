import { Pin, PinOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

/// vault 条目的行尾置顶按钮（注册表 rowAction 渲染）。
/// 行尾动作须阻止冒泡，避免顺带打开详情页
export function VaultRowPin({
  pinned,
  onPin,
}: {
  pinned: boolean;
  onPin: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          className="zap-row-pin text-muted-foreground hover:text-primary"
          aria-label={pinned ? "取消置顶" : "置顶"}
          onClick={(e) => {
            e.stopPropagation();
            onPin();
          }}
        >
          {pinned ? (
            <PinOff className="size-3.5" />
          ) : (
            <Pin className="size-3.5" />
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{pinned ? "取消置顶" : "置顶"}</TooltipContent>
    </Tooltip>
  );
}
