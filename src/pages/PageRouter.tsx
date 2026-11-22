import { useTranslation } from "react-i18next";
import {
  Activity,
  BarChart3,
  Download,
  Film,
  Home,
  PlusCircle,
  Scissors,
  type LucideIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { OnboardingPage } from "@/pages/OnboardingPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { useUiStore, type ScreenId } from "@/stores/uiStore";

interface ShellConfig {
  key: string;
  icon: LucideIcon;
  showQuickLinks?: boolean;
}

type PlaceholderScreen = Exclude<ScreenId, "settings" | "onboarding">;

const SHELL_MAP: Record<PlaceholderScreen, ShellConfig> = {
  home: { key: "home", icon: Home, showQuickLinks: true },
  "new-project": { key: "newProject", icon: PlusCircle, showQuickLinks: true },
  analysis: { key: "analysis", icon: Activity },
  timeline: { key: "timeline", icon: Scissors },
  review: { key: "review", icon: Film },
  export: { key: "export", icon: Download },
  costs: { key: "costs", icon: BarChart3 },
};

export function PageRouter() {
  const { t } = useTranslation();
  const activeScreen = useUiStore((s) => s.activeScreen);
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);

  if (activeScreen === "settings") return <SettingsPage />;
  if (activeScreen === "onboarding") return <OnboardingPage />;

  const cfg = SHELL_MAP[activeScreen];

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-fg">
          {t(`screens.${cfg.key}.title`)}
        </h1>
        <p className="text-sm text-fg-muted">{t(`screens.${cfg.key}.subtitle`)}</p>
      </header>

      <EmptyState
        icon={cfg.icon}
        title={t(`screens.${cfg.key}.emptyTitle`)}
        description={t(`screens.${cfg.key}.emptyDesc`)}
        actions={
          cfg.showQuickLinks && (
            <>
              <Button variant="primary" onClick={() => setActiveScreen("onboarding")}>
                {t("actions.openOnboarding")}
              </Button>
              <Button variant="secondary" onClick={() => setActiveScreen("settings")}>
                {t("actions.openSettings")}
              </Button>
            </>
          )
        }
      />
    </div>
  );
}
