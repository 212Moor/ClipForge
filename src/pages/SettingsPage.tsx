import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Database, Globe, Monitor, Moon, Palette, Sun } from "lucide-react";
import { commands, type ThemeMode } from "@/bindings";
import { GeminiSettingsCard } from "@/components/settings/GeminiSettingsCard";
import { PipelineSettingsCard } from "@/components/settings/PipelineSettingsCard";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useUiStore } from "@/stores/uiStore";
import type { AppLanguage } from "@/i18n";

function InfoTile({ label, value, wide }: { label: string; value: string; wide?: boolean }) {
  return (
    <div
      className={`rounded-xl border border-border/60 bg-surface-subtle/40 p-3 ${wide ? "sm:col-span-2" : ""}`}
    >
      <dt className="text-xs text-fg-subtle">{label}</dt>
      <dd className="mt-0.5 select-text break-all font-mono text-xs text-fg">{value}</dd>
    </div>
  );
}

export function SettingsPage() {
  const { t } = useTranslation();
  const theme = useUiStore((s) => s.theme);
  const setTheme = useUiStore((s) => s.setTheme);
  const language = useUiStore((s) => s.language);
  const setLanguage = useUiStore((s) => s.setLanguage);

  const bootstrapQuery = useQuery({
    queryKey: ["app-bootstrap"],
    queryFn: async () => {
      const res = await commands.getAppBootstrap();
      if (res.status === "error") {
        throw new Error(`${res.error.code} : ${res.error.message}`);
      }
      return res.data;
    },
  });

  const handleTheme = async (next: ThemeMode) => {
    await setTheme(next);
    toast.success(t("screens.settings.savedToast"));
  };

  const handleLanguage = async (next: AppLanguage) => {
    await setLanguage(next);
    toast.success(t("screens.settings.savedToast"));
  };

  const themeOptions: { value: ThemeMode; label: string; icon: typeof Moon }[] = [
    { value: "dark", label: t("theme.dark"), icon: Moon },
    { value: "light", label: t("theme.light"), icon: Sun },
    { value: "system", label: t("theme.system"), icon: Monitor },
  ];

  const langOptions: { value: AppLanguage; label: string }[] = [
    { value: "fr", label: t("language.fr") },
    { value: "en", label: t("language.en") },
  ];

  const info = bootstrapQuery.data;

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-fg">
          {t("screens.settings.title")}
        </h1>
        <p className="text-sm text-fg-muted">{t("screens.settings.subtitle")}</p>
      </header>

      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Palette className="h-4 w-4 text-accent-violet" aria-hidden="true" />
            <CardTitle>{t("screens.settings.appearanceCard")}</CardTitle>
          </div>
          <CardDescription>{t("screens.settings.appearanceDesc")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <span className="text-sm font-medium text-fg">{t("theme.label")}</span>
            <div className="flex flex-wrap gap-2">
              {themeOptions.map(({ value, label, icon: Icon }) => (
                <Button
                  key={value}
                  variant={theme === value ? "primary" : "secondary"}
                  size="sm"
                  aria-pressed={theme === value}
                  onClick={() => void handleTheme(value)}
                >
                  <Icon className="h-3.5 w-3.5" aria-hidden="true" />
                  {label}
                </Button>
              ))}
            </div>
          </div>

          <div className="h-px bg-border/60" />

          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <span className="flex items-center gap-2 text-sm font-medium text-fg">
              <Globe className="h-4 w-4 text-accent-cyan" aria-hidden="true" />
              {t("language.label")}
            </span>
            <div className="flex flex-wrap gap-2">
              {langOptions.map(({ value, label }) => (
                <Button
                  key={value}
                  variant={language === value ? "primary" : "secondary"}
                  size="sm"
                  aria-pressed={language === value}
                  onClick={() => void handleLanguage(value)}
                >
                  {label}
                </Button>
              ))}
            </div>
          </div>
        </CardContent>
      </Card>

      <GeminiSettingsCard />
      <PipelineSettingsCard />

      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Database className="h-4 w-4 text-accent-cyan" aria-hidden="true" />
            <CardTitle>{t("screens.settings.diagnosticsCard")}</CardTitle>
          </div>
          <CardDescription>{t("screens.settings.diagnosticsDesc")}</CardDescription>
        </CardHeader>
        <CardContent>
          {bootstrapQuery.isLoading && (
            <div className="space-y-2.5">
              <Skeleton className="h-5 w-2/3" />
              <Skeleton className="h-5 w-full" />
              <Skeleton className="h-5 w-4/5" />
            </div>
          )}

          {bootstrapQuery.isError && (
            <p className="text-sm text-status-danger">{(bootstrapQuery.error as Error).message}</p>
          )}

          {info && (
            <dl className="grid grid-cols-1 gap-3 text-sm sm:grid-cols-2">
              <InfoTile label={t("screens.settings.version")} value={`v${info.app_version}`} />
              <InfoTile label={t("screens.settings.vibrancy")} value={info.platform_vibrancy} />
              <InfoTile label={t("screens.settings.repo")} value={info.github_repo} wide />
              <InfoTile label={t("screens.settings.dataRoot")} value={info.data_root} wide />
              <InfoTile label={t("screens.settings.database")} value={info.db_path} wide />
              <InfoTile label={t("screens.settings.logs")} value={info.logs_dir} wide />
              <div className="rounded-xl border border-border/60 bg-surface-subtle/40 p-3 sm:col-span-2">
                <dt className="text-xs text-fg-subtle">{t("screens.settings.migrations")}</dt>
                <dd className="mt-1 flex flex-wrap gap-1.5">
                  {info.applied_migrations.map((m) => (
                    <Badge key={m.version} variant="success">
                      v{m.version} · {m.name}
                    </Badge>
                  ))}
                </dd>
              </div>
            </dl>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
