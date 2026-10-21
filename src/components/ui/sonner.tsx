import { Toaster as Sonner, type ToasterProps } from "sonner";

import { cn } from "@/lib/utils";

const Toaster = ({ className, ...props }: ToasterProps) => {
  return (
    <Sonner
      className={cn("z-[100]", className)}
      position="bottom-center"
      gap={6}
      offset={12}
      visibleToasts={2}
      closeButton={false}
      toastOptions={{
        classNames: {
          toast: cn(
            // sonner 容器固定 356px 居中、li 是 absolute 且 center 模式不设偏移，
            // 默认靠「li 与容器同宽」的巧合居中；这里收窄为内容宽后必须用
            // left/right:0 + margin auto 才是真居中（!w-auto 的静态定位会贴容器左缘）
            "!flex !items-center !gap-2 !left-0 !right-0 !mx-auto !w-fit !max-w-[92%] !rounded-lg !border !border-border",
            "!bg-popover/95 !backdrop-blur-sm !text-popover-foreground !shadow-lg !shadow-black/40",
            "!px-3 !py-2 !text-[12px] !font-medium !animate-toast-in"
          ),
          title: "!text-[12px] !font-medium",
          description: "!text-[11px] !text-muted-foreground",
          icon: "!size-3.5",
          success: "!text-success [&_[data-icon]]:!text-success",
          error: "!text-destructive [&_[data-icon]]:!text-destructive",
        },
      }}
      {...props}
    />
  );
};

export { Toaster };
