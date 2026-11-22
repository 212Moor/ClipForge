import type { ReactNode } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { Command } from "cmdk";
import { useTranslation } from "react-i18next";
import { Globe, Monitor, Moon, PanelLeft, Search, Sun } from "lucide-react";
import { FOOTER_NAV_ITEMS, MAIN_NAV_ITEMS } from "@/components/layout/Sidebar";
import { useUiStore } from "@/stores/uiStore";

const GROUP_CLASS = "px-1 py-1 text-xs font-medium text-fg-subtle";

function PaletteItem({
  onSelect,
  children,
  shortcut,
}: {
  onSelect: () => void;
  children: ReactNode;
  shortcut?: string;
}) {
  return (
    <Command.Item
      onSelect={onSelect}
      className="mt-0.5 flex cursor-pointer items-center gap-2.5 rounded-xl px-3 py-2 text-sm font-normal text-fg aria-selected:bg-accent-violet/20"
    >
      {children}
      {shortcut && <span className="ml-auto font-mono text-[10px] text-fg-subtle">{shortcut}</span>}
    </Command.Item>
  );
}

export function CommandPalette() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.commandPaletteOpen);
  const setOpen = useUiStore((s) => s.setCommandPaletteOpen);
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const setTheme = useUiStore((s) => s.setTheme);
  const setLanguage = useUiStore((s) => s.setLanguage);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);

  const runAndClose = (fn: () => void) => () => {
    fn();
    setOpen(false);
  };

  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm" />
        <Dialog.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-[18%] z-50 w-[min(560px,92vw)] -translate-x-1/2 overflow-hidden rounded-2xl border border-border-strong/70 bg-surface-elevated/95 shadow-2xl backdrop-blur-xl"
        >
          <Dialog.Title className="sr-only">{t("app.commandHint")}</Dialog.Title>
          <Command className="flex flex-col">
            <div className="flex items-center gap-2.5 border-b border-border/70 px-4 py-3">
              <Search className="h-4 w-4 text-fg-subtle" aria-hidden="true" />
              <Command.Input
                placeholder={t("commandPalette.placeholder")}
                className="w-full bg-transparent text-sm text-fg placeholder:text-fg-subtle focus:outline-none"
              />
              <kbd className="rounded-md border border-border px-1.5 py-0.5 font-mono text-[10px] text-fg-subtle">
                Échap
              </kbd>
            </div>

            <Command.List className="max-h-[340px] overflow-y-auto p-2">
              <Command.Empty className="py-8 text-center text-sm text-fg-muted">
                {t("commandPalette.empty")}
              </Command.Empty>

              <Command.Group heading={t("commandPalette.groupNavigation")} className={GROUP_CLASS}>
                {[...MAIN_NAV_ITEMS, ...FOOTER_NAV_ITEMS].map((item) => {
                  const Icon = item.icon;
                  return (
                    <PaletteItem
                      key={item.id}
                      onSelect={runAndClose(() => setActiveScreen(item.id))}
                      shortcut={item.shortcut}
                    >
                      <Icon className="h-4 w-4 text-accent-cyan" />
                      <span>{t(item.labelKey)}</span>
                    </PaletteItem>
                  );
                })}
              </Command.Group>

              <Command.Group
                heading={t("commandPalette.groupPreferences")}
                className={`mt-2 ${GROUP_CLASS}`}
              >
                <PaletteItem onSelect={runAndClose(() => void setTheme("dark"))}>
                  <Moon className="h-4 w-4 text-accent-violet" />
                  <span>{t("commandPalette.themeDark")}</span>
                </PaletteItem>
                <PaletteItem onSelect={runAndClose(() => void setTheme("light"))}>
                  <Sun className="h-4 w-4 text-accent-violet" />
                  <span>{t("commandPalette.themeLight")}</span>
                </PaletteItem>
                <PaletteItem onSelect={runAndClose(() => void setTheme("system"))}>
                  <Monitor className="h-4 w-4 text-accent-violet" />
                  <span>{t("commandPalette.themeSystem")}</span>
                </PaletteItem>
                <PaletteItem onSelect={runAndClose(() => void setLanguage("fr"))}>
                  <Globe className="h-4 w-4 text-accent-cyan" />
                  <span>{t("commandPalette.langFr")}</span>
                </PaletteItem>
                <PaletteItem onSelect={runAndClose(() => void setLanguage("en"))}>
                  <Globe className="h-4 w-4 text-accent-cyan" />
                  <span>{t("commandPalette.langEn")}</span>
                </PaletteItem>
              </Command.Group>

              <Command.Group heading={t("commandPalette.groupSystem")} className={`mt-2 ${GROUP_CLASS}`}>
                <PaletteItem onSelect={runAndClose(toggleSidebar)} shortcut="Ctrl+B">
                  <PanelLeft className="h-4 w-4 text-fg-muted" />
                  <span>{t("commandPalette.toggleSidebar")}</span>
                </PaletteItem>
              </Command.Group>
            </Command.List>
          </Command>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
