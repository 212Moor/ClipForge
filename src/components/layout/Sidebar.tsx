import { useTranslation } from "react-i18next";
import { motion } from "framer-motion";
import {
  Activity,
  BarChart3,
  Download,
  Film,
  Home,
  PackageCheck,
  PanelLeftClose,
  PanelLeftOpen,
  PlusCircle,
  Scissors,
  Settings,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useUiStore, type ScreenId } from "@/stores/uiStore";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

interface NavItem {
  id: ScreenId;
  labelKey: string;
  icon: LucideIcon;
  shortcut?: string;
}

export const MAIN_NAV_ITEMS: NavItem[] = [
  { id: "home", labelKey: "nav.home", icon: Home, shortcut: "Alt+1" },
  { id: "new-project", labelKey: "nav.newProject", icon: PlusCircle, shortcut: "Alt+2" },
  { id: "analysis", labelKey: "nav.analysis", icon: Activity, shortcut: "Alt+3" },
  { id: "timeline", labelKey: "nav.timeline", icon: Scissors, shortcut: "Alt+4" },
  { id: "review", labelKey: "nav.review", icon: Film, shortcut: "Alt+5" },
  { id: "export", labelKey: "nav.export", icon: Download, shortcut: "Alt+6" },
  { id: "costs", labelKey: "nav.costs", icon: BarChart3, shortcut: "Alt+7" },
];

export const FOOTER_NAV_ITEMS: NavItem[] = [
  { id: "onboarding", labelKey: "nav.onboarding", icon: PackageCheck },
  { id: "settings", labelKey: "nav.settings", icon: Settings, shortcut: "Ctrl+," },
];

export function Sidebar() {
  const { t } = useTranslation();
  const activeScreen = useUiStore((s) => s.activeScreen);
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);

  const renderItem = (item: NavItem) => {
    const Icon = item.icon;
    const active = activeScreen === item.id;
    const label = t(item.labelKey);

    const button = (
      <button
        key={item.id}
        type="button"
        onClick={() => setActiveScreen(item.id)}
        aria-current={active ? "page" : undefined}
        aria-label={collapsed ? label : undefined}
        className={cn(
          "relative flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-sm font-medium transition-colors",
          collapsed && "justify-center px-0",
          active ? "text-fg" : "text-fg-muted hover:bg-surface-subtle/60 hover:text-fg",
        )}
      >
        {active && (
          <motion.span
            layoutId="cf-active-nav"
            transition={{ type: "spring", stiffness: 420, damping: 34 }}
            className="absolute inset-0 rounded-xl border border-accent-violet/35 bg-gradient-to-r from-accent-violet/20 to-accent-cyan/10"
          />
        )}
        <Icon
          className={cn("relative z-10 h-4 w-4 shrink-0", active ? "text-accent-cyan" : "text-fg-muted")}
          aria-hidden="true"
        />
        {!collapsed && (
          <>
            <span className="relative z-10 truncate">{label}</span>
            {item.shortcut && (
              <span className="relative z-10 ml-auto font-mono text-[10px] text-fg-subtle">
                {item.shortcut}
              </span>
            )}
          </>
        )}
      </button>
    );

    if (!collapsed) return button;

    return (
      <Tooltip key={item.id} delayDuration={180}>
        <TooltipTrigger asChild>{button}</TooltipTrigger>
        <TooltipContent side="right" className="flex items-center gap-2">
          <span>{label}</span>
          {item.shortcut && (
            <kbd className="rounded bg-surface-subtle px-1 py-0.5 font-mono text-[10px] text-fg-muted">
              {item.shortcut}
            </kbd>
          )}
        </TooltipContent>
      </Tooltip>
    );
  };

  return (
    <aside
      aria-label={t("nav.label")}
      className={cn(
        "flex h-full shrink-0 flex-col justify-between border-r border-border/60 bg-surface-elevated/35 p-2.5 transition-[width] duration-200",
        collapsed ? "w-[64px]" : "w-[228px]",
      )}
    >
      <nav className="flex flex-col gap-1">{MAIN_NAV_ITEMS.map(renderItem)}</nav>

      <div className="flex flex-col gap-1 border-t border-border/60 pt-2.5">
        {FOOTER_NAV_ITEMS.map(renderItem)}

        <button
          type="button"
          onClick={toggleSidebar}
          aria-label={collapsed ? t("nav.expandSidebar") : t("nav.collapseSidebar")}
          className={cn(
            "mt-1 flex items-center gap-3 rounded-xl px-2.5 py-2 text-xs text-fg-subtle transition-colors hover:bg-surface-subtle/60 hover:text-fg",
            collapsed && "justify-center px-0",
          )}
        >
          {collapsed ? (
            <PanelLeftOpen className="h-4 w-4" aria-hidden="true" />
          ) : (
            <>
              <PanelLeftClose className="h-4 w-4" aria-hidden="true" />
              <span>{t("nav.collapseSidebar")}</span>
            </>
          )}
        </button>
      </div>
    </aside>
  );
}
