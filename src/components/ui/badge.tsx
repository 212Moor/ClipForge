import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium",
  {
    variants: {
      variant: {
        default: "border-border bg-surface-subtle/70 text-fg-muted",
        violet: "border-accent-violet/40 bg-accent-violet/15 text-accent-violet",
        cyan: "border-accent-cyan/40 bg-accent-cyan/15 text-accent-cyan",
        success: "border-status-success/40 bg-status-success/15 text-status-success",
        warning: "border-status-warning/40 bg-status-warning/15 text-status-warning",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof badgeVariants> {}

export function Badge({ className, variant, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ variant }), className)} {...props} />;
}
