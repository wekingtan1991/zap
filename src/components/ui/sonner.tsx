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
            "!flex !items-center !gap-2 !w-auto !max-w-[92%] !self-center !rounded-lg !border !border-border",
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
