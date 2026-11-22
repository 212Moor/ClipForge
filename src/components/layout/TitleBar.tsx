import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { Command, Copy, Minus, Monitor, Moon, Sparkles, Square, Sun, X } from "lucide-react";
import { useUiStore } from "@/stores/uiStore";
import type { ThemeMode } from "@/bindings";

const THEME_CYCLE: ThemeMode[] = ["dark", "light", "system"];

export function TitleBar() {
  const { t } = useTranslation();
  const theme = useUiStore((s) => s.theme);
  const setTheme = useUiStore((s) => s.setTheme);
  const setCommandPaletteOpen = useUiStore((s) => s.setCommandPaletteOpen);
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    const sync = () => void appWindow.isMaximized().then(setIsMaximized).catch(() => undefined);
    sync();
    const unlistenPromise = appWindow.onResized(sync);
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const cycleTheme = () => {
    const next = THEME_CYCLE[(THEME_CYCLE.indexOf(theme) + 1) % THEME_CYCLE.length];
    void setTheme(next);
  };

  const ThemeIcon = theme === "dark" ? Moon : theme === "light" ? Sun : Monitor;
  const themeLabel = `${t("theme.label")} : ${t(`theme.${theme}`)}`;
  const controlClass =
    "flex h-8 w-10 items-center justify-center rounded-md text-fg-muted transition-colors hover:bg-surface-subtle/80 hover:text-fg";

  return (
    <header className="flex h-11 shrink-0 select-none items-center justify-between border-b border-border/60 bg-surface-elevated/45 px-3">
      <div data-tauri-drag-region className="flex h-full flex-1 items-center gap-2.5">
        <div
          data-tauri-drag-region
          className="flex h-6 w-6 items-center justify-center rounded-lg bg-gradient-to-br from-accent-violet to-accent-cyan text-white shadow-sm"
        >
          <Sparkles className="pointer-events-none h-3.5 w-3.5" aria-hidden="true" />
        </div>
        <span data-tauri-drag-region className="text-sm font-semibold tracking-tight text-fg">
          {t("app.name")}
        </span>
        <span data-tauri-drag-region className="hidden text-xs text-fg-subtle sm:inline">
          {t("app.tagline")}
        </span>
      </div>

      <div className="mr-2 flex items-center gap-1.5">
        <button
          type="button"
          onClick={() => setCommandPaletteOpen(true)}
          className="flex h-7 items-center gap-2 rounded-lg border border-border/80 bg-surface-subtle/60 px-2.5 text-xs text-fg-muted transition-colors hover:border-border-strong hover:text-fg"
        >
          <Command className="h-3.5 w-3.5 text-accent-cyan" aria-hidden="true" />
          <span>{t("app.commandHint")}</span>
          <kbd className="rounded bg-surface-elevated/90 px-1.5 py-0.5 font-mono text-[10px] text-fg-subtle">
            Ctrl+K
          </kbd>
        </button>

        <button
          type="button"
          onClick={cycleTheme}
          aria-label={themeLabel}
          title={themeLabel}
          className="flex h-7 w-7 items-center justify-center rounded-lg text-fg-muted transition-colors hover:bg-surface-subtle/80 hover:text-fg"
        >
          <ThemeIcon className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </div>

      <div className="flex items-center">
        <button
          type="button"
          onClick={() => void getCurrentWindow().minimize()}
          aria-label={t("window.minimize")}
          className={controlClass}
        >
          <Minus className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          onClick={() => void getCurrentWindow().toggleMaximize()}
          aria-label={isMaximized ? t("window.restore") : t("window.maximize")}
          className={controlClass}
        >
          {isMaximized ? <Copy className="h-3 w-3" /> : <Square className="h-3 w-3" />}
        </button>
        <button
          type="button"
          onClick={() => void getCurrentWindow().close()}
          aria-label={t("window.close")}
          className="flex h-8 w-10 items-center justify-center rounded-md text-fg-muted transition-colors hover:bg-status-danger hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </header>
  );
}
