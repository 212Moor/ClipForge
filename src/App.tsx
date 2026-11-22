import { useEffect } from "react";
import { commands } from "@/bindings";
import { AppShell } from "@/components/layout/AppShell";
import { PageRouter } from "@/pages/PageRouter";
import { useUiStore } from "@/stores/uiStore";

export default function App() {
  const hydratePreferences = useUiStore((s) => s.hydratePreferences);
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      await hydratePreferences();

      // Premier lancement : tant que les 4 outils ne sont pas installés, on ouvre l'écran d'installation.
      const res = await commands.getSidecarsOverview(false);
      if (!cancelled && res.status === "ok" && !res.data.all_ready) {
        setActiveScreen("onboarding");
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [hydratePreferences, setActiveScreen]);

  return (
    <AppShell>
      <PageRouter />
    </AppShell>
  );
}
