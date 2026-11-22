import { create } from "zustand";
import { commands, type ThemeMode } from "@/bindings";
import i18n, { type AppLanguage } from "@/i18n";

export type ScreenId =
  | "home"
  | "new-project"
  | "analysis"
  | "timeline"
  | "review"
  | "export"
  | "costs"
  | "settings"
  | "onboarding";

interface UiState {
  activeScreen: ScreenId;
  sidebarCollapsed: boolean;
  commandPaletteOpen: boolean;
  theme: ThemeMode;
  language: AppLanguage;
  hydrated: boolean;
  setActiveScreen: (screen: ScreenId) => void;
  toggleSidebar: () => void;
  setCommandPaletteOpen: (open: boolean) => void;
  setTheme: (theme: ThemeMode) => Promise<void>;
  setLanguage: (language: AppLanguage) => Promise<void>;
  hydratePreferences: () => Promise<void>;
}

function applyDocumentTheme(theme: ThemeMode) {
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const isDark = theme === "dark" || (theme === "system" && prefersDark);
  document.documentElement.classList.toggle("dark", isDark);
}

function parseStored<T extends string>(raw: string | null, allowed: readonly T[]): T | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as T;
    return allowed.includes(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export const useUiStore = create<UiState>((set) => ({
  activeScreen: "home",
  sidebarCollapsed: false,
  commandPaletteOpen: false,
  theme: "dark",
  language: "fr",
  hydrated: false,

  setActiveScreen: (screen) => set({ activeScreen: screen }),

  toggleSidebar: () => set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),

  setCommandPaletteOpen: (open) => set({ commandPaletteOpen: open }),

  setTheme: async (theme) => {
    set({ theme });
    applyDocumentTheme(theme);
    await Promise.allSettled([
      commands.applyWindowTheme(theme),
      commands.setSetting("ui.theme", JSON.stringify(theme)),
    ]);
  },

  setLanguage: async (language) => {
    set({ language });
    document.documentElement.lang = language;
    await i18n.changeLanguage(language);
    await commands.setSetting("ui.language", JSON.stringify(language));
  },

  hydratePreferences: async () => {
    const [themeRes, langRes] = await Promise.all([
      commands.getSetting("ui.theme"),
      commands.getSetting("ui.language"),
    ]);

    const theme =
      parseStored<ThemeMode>(themeRes.status === "ok" ? themeRes.data : null, [
        "dark",
        "light",
        "system",
      ]) ?? "dark";
    const language =
      parseStored<AppLanguage>(langRes.status === "ok" ? langRes.data : null, ["fr", "en"]) ??
      "fr";

    applyDocumentTheme(theme);
    document.documentElement.lang = language;
    await i18n.changeLanguage(language);
    await commands.applyWindowTheme(theme);

    set({ theme, language, hydrated: true });
  },
}));
