import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  Cpu,
  Download,
  FolderInput,
  RefreshCw,
  ShieldCheck,
  Wrench,
} from "lucide-react";
import {
  commands,
  events,
  type ProgressEvent,
  type SidecarId,
  type WhisperVariant,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useUiStore } from "@/stores/uiStore";

// Doit rester identique à `job_id_for` côté Rust : "sidecar-" + SidecarId::dir_name().
const JOB_KEYS: Record<SidecarId, string> = {
  ffmpeg: "sidecar-ffmpeg",
  yt_dlp: "sidecar-yt-dlp",
  twitch_downloader: "sidecar-twitch-downloader",
  whisper_cpp: "sidecar-whisper-cpp",
};

export function OnboardingPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);

  const [tryRemote, setTryRemote] = useState(false);
  const [busyTools, setBusyTools] = useState<Partial<Record<SidecarId, boolean>>>({});
  const [importingOffline, setImportingOffline] = useState(false);
  const [progressByJob, setProgressByJob] = useState<Record<string, ProgressEvent>>({});

  const overviewQuery = useQuery({
    queryKey: ["sidecars-overview", tryRemote],
    queryFn: async () => {
      const res = await commands.getSidecarsOverview(tryRemote);
      if (res.status === "error") {
        throw new Error(`${res.error.code} : ${res.error.message}`);
      }
      return res.data;
    },
  });

  // Progression poussée par Rust (aucun polling).
  useEffect(() => {
    const unlistenPromise = events.progressEvent.listen((ev) => {
      if (ev.payload.scope !== "sidecars") return;
      setProgressByJob((prev) => ({ ...prev, [ev.payload.job_id]: ev.payload }));
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const refreshOverview = async () => {
    await queryClient.invalidateQueries({ queryKey: ["sidecars-overview"] });
  };

  const handleCheckRemote = async () => {
    if (tryRemote) {
      await refreshOverview();
    } else {
      setTryRemote(true); // la clé de requête change, ce qui relance la requête
    }
  };

  const handleSelectVariant = async (variant: WhisperVariant) => {
    const res = await commands.setWhisperVariant(variant);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    await refreshOverview();
  };

  const handleInstallOne = async (id: SidecarId) => {
    setBusyTools((prev) => ({ ...prev, [id]: true }));
    try {
      const variant = id === "whisper_cpp" ? (overviewQuery.data?.active_whisper_variant ?? null) : null;
      const res = await commands.installSidecar(id, variant);
      if (res.status === "error") {
        toast.error(res.error.message);
      } else if (!res.data.success) {
        toast.warning(res.data.message);
      } else {
        toast.success(res.data.message);
      }
      await refreshOverview();
    } finally {
      setBusyTools((prev) => ({ ...prev, [id]: false }));
    }
  };

  const handleInstallAll = async () => {
    const items = overviewQuery.data?.items ?? [];
    for (const item of items) {
      if (!item.is_installed_and_present || item.needs_update) {
        await handleInstallOne(item.id);
      }
    }
  };

  const handleSelfTest = async (id: SidecarId) => {
    setBusyTools((prev) => ({ ...prev, [id]: true }));
    try {
      const res = await commands.testSidecar(id);
      if (res.status === "error") {
        toast.error(res.error.message);
      } else {
        toast.success(res.data.message);
      }
      await refreshOverview();
    } finally {
      setBusyTools((prev) => ({ ...prev, [id]: false }));
    }
  };

  const handleOfflineImport = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("sidecars.importOffline"),
    });
    if (!selected || typeof selected !== "string") return;

    setImportingOffline(true);
    try {
      const res = await commands.importSidecarsOffline(selected);
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      const okCount = res.data.filter((r) => r.success).length;
      if (okCount > 0) {
        toast.success(`${t("sidecars.offlineSuccess")} (${okCount}/${res.data.length})`);
      } else {
        toast.warning(res.data[0]?.message ?? t("sidecars.offlineSuccess"));
      }
      await refreshOverview();
    } finally {
      setImportingOffline(false);
    }
  };

  const data = overviewQuery.data;
  const anyHashMissing = data?.items.some((i) => !i.manifest_ready) ?? false;
  const anyBusy = Object.values(busyTools).some(Boolean);

  return (
    <div className="space-y-6">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">
            {t("screens.onboarding.title")}
          </h1>
          <p className="max-w-xl text-sm text-fg-muted">{t("screens.onboarding.subtitle")}</p>
        </div>

        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void handleCheckRemote()}
            disabled={overviewQuery.isFetching}
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${overviewQuery.isFetching ? "animate-spin" : ""}`}
              aria-hidden="true"
            />
            {t("sidecars.checkRemote")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void handleOfflineImport()}
            disabled={importingOffline || anyBusy}
          >
            <FolderInput className="h-3.5 w-3.5" aria-hidden="true" />
            {t("sidecars.importOffline")}
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={() => void handleInstallAll()}
            disabled={!data || anyBusy || importingOffline}
          >
            <Download className="h-3.5 w-3.5" aria-hidden="true" />
            {t("sidecars.installAll")}
          </Button>
        </div>
      </header>

      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <Cpu className="h-4 w-4 text-accent-cyan" aria-hidden="true" />
              <CardTitle>{t("sidecars.gpuTitle")}</CardTitle>
            </div>
            {data && (
              <Badge variant={data.manifest_source === "remote_signed" ? "success" : "violet"}>
                <ShieldCheck className="h-3 w-3" aria-hidden="true" />
                {data.manifest_source === "remote_signed"
                  ? t("sidecars.manifestRemote")
                  : t("sidecars.manifestEmbedded")}
              </Badge>
            )}
          </div>
          {data?.manifest_warning && <CardDescription>{data.manifest_warning}</CardDescription>}
        </CardHeader>

        <CardContent className="space-y-4">
          {overviewQuery.isLoading && (
            <div className="space-y-2">
              <Skeleton className="h-5 w-1/2" />
              <Skeleton className="h-8 w-full" />
            </div>
          )}

          {overviewQuery.isError && (
            <p className="text-sm text-status-danger">{(overviewQuery.error as Error).message}</p>
          )}

          {data && (
            <>
              <div className="flex flex-wrap items-center gap-2 text-xs text-fg-muted">
                {data.gpu.adapters.length > 0 ? (
                  data.gpu.adapters.map((name) => (
                    <Badge key={name} variant="default">
                      {name}
                    </Badge>
                  ))
                ) : (
                  <span>{t("sidecars.gpuNone")}</span>
                )}
                {data.gpu.has_cuda && <Badge variant="cyan">CUDA</Badge>}
                {data.gpu.has_vulkan && <Badge variant="violet">Vulkan</Badge>}
              </div>

              <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                <span className="text-sm font-medium text-fg">{t("sidecars.whisperVariant")}</span>
                <div className="flex flex-wrap gap-2">
                  {data.available_whisper_variants.map((variant) => (
                    <Button
                      key={variant}
                      size="sm"
                      variant={variant === data.active_whisper_variant ? "primary" : "secondary"}
                      aria-pressed={variant === data.active_whisper_variant}
                      onClick={() => void handleSelectVariant(variant)}
                    >
                      <span className="uppercase">{variant}</span>
                      {variant === data.recommended_whisper_variant && (
                        <span className="rounded bg-black/20 px-1.5 py-0.5 text-[10px]">
                          {t("sidecars.recommended")}
                        </span>
                      )}
                    </Button>
                  ))}
                </div>
              </div>
            </>
          )}
        </CardContent>
      </Card>

      {anyHashMissing && (
        <div
          role="note"
          className="flex items-start gap-3 rounded-2xl border border-status-warning/40 bg-status-warning/10 px-4 py-3 text-sm text-fg"
        >
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-status-warning" aria-hidden="true" />
          <p>{t("sidecars.hashHelp")}</p>
        </div>
      )}

      <div className="grid grid-cols-1 gap-3">
        {overviewQuery.isLoading && (
          <>
            <Skeleton className="h-24 w-full rounded-2xl" />
            <Skeleton className="h-24 w-full rounded-2xl" />
            <Skeleton className="h-24 w-full rounded-2xl" />
            <Skeleton className="h-24 w-full rounded-2xl" />
          </>
        )}

        {data?.items.map((item) => {
          const isBusy = Boolean(busyTools[item.id]);
          const progress = progressByJob[JOB_KEYS[item.id]];

          return (
            <Card key={item.id} className="p-4">
              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="text-sm font-semibold text-fg">{item.display_name}</h3>
                    <Badge variant="default" className="font-mono text-[11px]">
                      {item.target_version} · {item.target_variant}
                    </Badge>

                    {item.is_installed_and_present && !item.needs_update && (
                      <Badge variant="success">
                        <CheckCircle2 className="h-3 w-3" aria-hidden="true" />
                        {t("sidecars.statusReady")}
                      </Badge>
                    )}
                    {item.is_installed_and_present && item.needs_update && (
                      <Badge variant="warning">{t("sidecars.statusUpdate")}</Badge>
                    )}
                    {!item.is_installed_and_present && (
                      <Badge variant={item.manifest_ready ? "default" : "warning"}>
                        {item.manifest_ready
                          ? t("sidecars.statusMissing")
                          : t("sidecars.statusHashMissing")}
                      </Badge>
                    )}
                  </div>

                  {item.installed && (
                    <p className="break-all font-mono text-[11px] text-fg-subtle">
                      SHA-256 : {item.installed.executable_sha256.slice(0, 24)}…
                    </p>
                  )}
                </div>

                <div className="flex items-center gap-2">
                  {item.is_installed_and_present && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={isBusy}
                      onClick={() => void handleSelfTest(item.id)}
                    >
                      <Wrench className="h-3.5 w-3.5" aria-hidden="true" />
                      {t("sidecars.retest")}
                    </Button>
                  )}

                  <Button
                    size="sm"
                    variant={
                      item.is_installed_and_present && !item.needs_update ? "secondary" : "primary"
                    }
                    disabled={isBusy || !item.manifest_ready}
                    onClick={() => void handleInstallOne(item.id)}
                  >
                    <Download className="h-3.5 w-3.5" aria-hidden="true" />
                    {item.needs_update ? t("sidecars.update") : t("sidecars.install")}
                  </Button>
                </div>
              </div>

              {isBusy && progress && (
                <div className="mt-3 space-y-1.5">
                  <div className="flex items-center justify-between text-xs text-fg-muted">
                    <span>{progress.message}</span>
                    <span className="font-mono">{Math.round(progress.percent)} %</span>
                  </div>
                  <div
                    role="progressbar"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round(progress.percent)}
                    className="h-1.5 w-full overflow-hidden rounded-full bg-surface-subtle"
                  >
                    <div
                      className="h-full rounded-full bg-gradient-to-r from-accent-violet to-accent-cyan transition-[width] duration-150"
                      style={{ width: `${Math.min(100, progress.percent)}%` }}
                    />
                  </div>
                </div>
              )}
            </Card>
          );
        })}
      </div>

      {data && (
        <div className="flex flex-col items-start justify-between gap-3 rounded-2xl border border-border/70 bg-surface-elevated/55 px-5 py-4 sm:flex-row sm:items-center">
          <p className="text-sm text-fg-muted">
            {data.all_ready ? t("sidecars.allReadyBanner") : t("sidecars.missingBanner")}
          </p>
          <Button
            variant={data.all_ready ? "primary" : "secondary"}
            onClick={() => setActiveScreen("home")}
          >
            {t("sidecars.continueToApp")}
            <ArrowRight className="h-4 w-4" aria-hidden="true" />
          </Button>
        </div>
      )}
    </div>
  );
}
