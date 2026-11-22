import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Card } from "@/components/ui/card";

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description: string;
  actions?: ReactNode;
}

export function EmptyState({ icon: Icon, title, description, actions }: EmptyStateProps) {
  return (
    <Card className="flex flex-col items-center justify-center gap-4 px-8 py-14 text-center">
      <div className="flex h-14 w-14 items-center justify-center rounded-2xl border border-accent-violet/30 bg-gradient-to-br from-accent-violet/15 to-accent-cyan/15 text-accent-cyan">
        <Icon className="h-6 w-6" aria-hidden="true" />
      </div>
      <div className="max-w-md space-y-1.5">
        <h3 className="text-base font-semibold text-fg">{title}</h3>
        <p className="text-sm leading-relaxed text-fg-muted">{description}</p>
      </div>
      {actions && (
        <div className="flex flex-wrap items-center justify-center gap-2 pt-1">{actions}</div>
      )}
    </Card>
  );
}
