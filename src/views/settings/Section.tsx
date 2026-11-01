/// 设置页分区壳（审计 E3）：边框 + 图标标题头 + 内容——十个分区的同构外壳，
/// 收拢此前逐分区复制的两层 div 样板
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

export function Section({
  icon: Icon,
  title,
  children,
}: {
  icon: LucideIcon;
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="mt-3 border-t border-border pt-3">
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
        <Icon className="size-3 text-primary/75" />
        {title}
      </div>
      {children}
    </div>
  );
}
