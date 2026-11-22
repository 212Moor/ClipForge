import { useEffect } from "react";
import type { ReactNode } from "react";
import { MotionConfig, motion } from "framer-motion";
import { Toaster } from "sonner";
import { TitleBar } from "@/components/layout/TitleBar";
import { MAIN_NAV_ITEMS, Sidebar } from "@/components/layout/Sidebar";
import { CommandPalette } from "@/components/layout/CommandPalette";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/stores/uiStore";

export function AppShell({ children }: { children: ReactNode }) {
  const activeScreen = useUiStore((s) => s.activeScreen);
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const setCommandPaletteOpen = useUiStore((s) => s.setCommandPaletteOpen);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);
  const theme = useUiStore((s) => s.theme);

  // Raccourcis globaux : Ctrl+K, Ctrl+B, Ctrl+, et Alt+1…7.
  // Alt+chiffre lit la touche physique (e.code) pour fonctionner aussi en AZERTY,
  // où la rangée du haut produit &, é, ", ', (, -, è sans Maj.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;

      if (mod && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCommandPaletteOpen(true);
        return;
      }
      if (mod && !e.altKey && e.key === ",") {
        e.preventDefault();
        setActiveScreen("settings");
        return;
      }
      if (mod && !e.altKey && e.key.toLowerCase() === "b") {
        e.preventDefault();
        toggleSidebar();
        return;
      }

      if (e.altKey && !e.ctrlKey && !e.metaKey) {
        const match = /^(?:Digit|Numpad)([1-7])$/.exec(e.code);
        if (match) {
          const target = MAIN_NAV_ITEMS[Number(match[1]) - 1];
          if (target) {
            e.preventDefault();
            setActiveScreen(target.id);
          }
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [setActiveScreen, setCommandPaletteOpen, toggleSidebar]);

  // Suivi en direct du thème Windows quand le mode « système » est actif.
  useEffect(() => {
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const listener = (ev: MediaQueryListEvent) => {
      document.documentElement.classList.toggle("dark", ev.matches);
    };
    mq.addEventListener("change", listener);
    return () => mq.removeEventListener("change", listener);
  }, [theme]);

  return (
    <MotionConfig reducedMotion="user">
      <TooltipProvider>
        <div className="cf-mica-shell flex h-full w-full flex-col overflow-hidden text-fg">
          <TitleBar />

          <div className="flex min-h-0 flex-1 overflow-hidden">
            <Sidebar />

            <main className="relative flex-1 overflow-y-auto px-7 py-6">
              <motion.div
                key={activeScreen}
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.16, ease: "easeOut" }}
                className="mx-auto w-full max-w-5xl"
              >
                {children}
              </motion.div>
            </main>
          </div>

          <CommandPalette />
          <Toaster
            position="bottom-right"
            richColors
            theme={theme}
            toastOptions={{
              className: "!rounded-xl !border !border-border !bg-surface-elevated !text-fg",
            }}
          />
        </div>
      </TooltipProvider>
    </MotionConfig>
  );
}
