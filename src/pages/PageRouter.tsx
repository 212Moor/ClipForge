import { AnalysisPage } from "@/pages/AnalysisPage";
import { CostsPage } from "@/pages/CostsPage";
import { ExportPage } from "@/pages/ExportPage";
import { HomePage } from "@/pages/HomePage";
import { NewProjectPage } from "@/pages/NewProjectPage";
import { OnboardingPage } from "@/pages/OnboardingPage";
import { ReviewPage } from "@/pages/ReviewPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { TimelinePage } from "@/pages/TimelinePage";
import { useUiStore } from "@/stores/uiStore";

export function PageRouter() {
  const activeScreen = useUiStore((s) => s.activeScreen);

  switch (activeScreen) {
    case "home":
      return <HomePage />;
    case "new-project":
      return <NewProjectPage />;
    case "analysis":
      return <AnalysisPage />;
    case "timeline":
      return <TimelinePage />;
    case "review":
      return <ReviewPage />;
    case "export":
      return <ExportPage />;
    case "costs":
      return <CostsPage />;
    case "settings":
      return <SettingsPage />;
    case "onboarding":
      return <OnboardingPage />;
  }
}
