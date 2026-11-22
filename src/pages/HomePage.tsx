import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  Activity,
  ArrowRight,
  Clock,
  Coins,
  Database,
  Film,
  FlaskConical,
  Link2,
  Scissors,
  Sparkles,
  Trash2,
} from "lucide-react";
import { commands, type ProjectSummary } from "@/bindings";
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
import { formatCurrency, formatDuration, formatTokens } from "@/lib/format";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

const STATUS_META: Record<string, { label: string; variant: "default" | "violet" | "cyan" | "success" | "warning" }> = {
  draft: { label: "Brouillon", variant: "default" },
  running: { label: "En cours", variant: "cyan" },
  paused: { label: "En pause", variant: "warning" },
  budget_paused: { label: "Plafond atteint", variant: "warning" },
  eco_batch_pending: { label: "Batch Éco en attente", variant: "violet" },
  local_done: { label: "Local terminé", variant: "cyan" },
  ready: { label: "Prêt", variant: "success" },
  failed: { label: "Erreur", variant: "warning" },
};

export function HomePage() {
  const queryClient = useQueryClient();
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);
  const setDraftVodUrl = useProjectStore((s) => s.setDraftVodUrl);

  const [urlInput, setUrlInput] = useState("");
  const [creatingDemo, setCreatingDemo] = useState(false);

  const projectsQuery = useQuery({
    queryKey: ["projects"],
    queryFn: async () => {
      const res = await commands.listProjectsCmd();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const gaugeQuery = useQuery({
    queryKey: ["token-gauge"],
    queryFn: async () => {
      const res = await commands.getTokenGauge();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const handleGoNewProject = (e: React.FormEvent) => {
    e.preventDefault();
    const cleaned = urlInput.trim();
    if (!cleaned) {
      setActiveScreen("new-project");
      return;
    }
    setDraftVodUrl(cleaned);
    setActiveScreen("new-project");
  };

  const handleDrop = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    const dropped = e.dataTransfer.getData("text/uri-list") || e.dataTransfer.getData("text/plain");
    if (dropped.trim()) {
      setUrlInput(dropped.trim());
      setDraftVodUrl(dropped.trim());
      setActiveScreen("new-project");
    }
  };

  const handleCreateDemo = async () => {
    setCreatingDemo(true);
    try {
      const res = await commands.createDemoProjectCmd();
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      setActiveProjectId(res.data);
      toast.success("Projet démo 10 min créé (audio + chat + zone mutée prêts).");
      setActiveScreen("analysis");
    } finally {
      setCreatingDemo(false);
    }
  };

  const handleDelete = async (project: ProjectSummary) => {
    const res = await commands.deleteProjectCmd(project.id);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success(`Projet « ${project.name} » supprimé.`);
    await queryClient.invalidateQueries({ queryKey: ["projects"] });
  };

  const openProject = (project: ProjectSummary, target: "analysis" | "timeline") => {
    setActiveProjectId(project.id);
    setActiveScreen(target);
  };

  const gauge = gaugeQuery.data;
  const cacheRate =
    gauge && gauge.total_calls > 0
      ? Math.round((gauge.total_cache_hits / gauge.total_calls) * 100)
      : 0;

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-fg">Accueil</h1>
        <p className="text-sm text-fg-muted">
          Collez une URL de rediffusion Twitch : signaux locaux d'abord, Gemini uniquement pour le tri éditorial.
        </p>
      </header>

      {/* Champ géant URL VOD + glisser-déposer */}
      <Card
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
        className="border-accent-violet/35 bg-gradient-to-br from-accent-violet/10 via-surface-elevated/75 to-accent-cyan/10"
      >
        <CardContent className="p-6">
          <form onSubmit={handleGoNewProject} className="space-y-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2 text-sm font-semibold text-fg">
                <Sparkles className="h-4 w-4 text-accent-cyan" />
                <span>Nouvelle rediffusion Twitch</span>
              </div>
              <Button
                variant="outline"
                size="sm"
                disabled={creatingDemo}
                onClick={() => void handleCreateDemo()}
              >
                <FlaskConical className="h-3.5 w-3.5 text-accent-violet" />
                {creatingDemo ? "Création du jeu 10 min…" : "Tester avec la démo 10 min (0 token)"}
              </Button>
            </div>

            <div className="flex flex-col gap-2.5 sm:flex-row">
              <div className="relative flex-1">
                <Link2 className="pointer-events-none absolute left-3.5 top-1/2 h-4 w-4 -translate-y-1/2 text-fg-subtle" />
                <input
                  type="text"
                  value={urlInput}
                  onChange={(e) => setUrlInput(e.target.value)}
                  placeholder="Colle l'URL d'une VOD (ex. https://www.twitch.tv/videos/1234567890) ou glisse-dépose…"
                  className="h-12 w-full rounded-xl border border-border-strong/70 bg-surface-base/80 pl-10 pr-4 text-sm text-fg placeholder:text-fg-subtle focus:border-accent-cyan focus:outline-none"
                />
              </div>
              <Button type="submit" variant="primary" size="lg" className="h-12 px-6">
                Configurer l'analyse
                <ArrowRight className="h-4 w-4" />
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Jauge de consommation de tokens (Jour / Mois / Cache) */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Card className="p-4">
          <div className="flex items-center justify-between text-xs text-fg-subtle">
            <span>Tokens aujourd'hui</span>
            <Coins className="h-3.5 w-3.5 text-accent-cyan" />
          </div>
          {gaugeQuery.isLoading ? (
            <Skeleton className="mt-2 h-7 w-28" />
          ) : (
            <div className="mt-2 space-y-0.5">
              <p className="font-mono text-xl font-semibold text-fg">
                {formatTokens((gauge?.today_input_tokens ?? 0) + (gauge?.today_output_tokens ?? 0))}
              </p>
              <p className="text-xs text-fg-muted">
                ≈ {formatCurrency((gauge?.today_cost_usd ?? 0) * 0.92, "EUR")}
              </p>
            </div>
          )}
        </Card>

        <Card className="p-4">
          <div className="flex items-center justify-between text-xs text-fg-subtle">
            <span>Tokens ce mois-ci</span>
            <Activity className="h-3.5 w-3.5 text-accent-violet" />
          </div>
          {gaugeQuery.isLoading ? (
            <Skeleton className="mt-2 h-7 w-28" />
          ) : (
            <div className="mt-2 space-y-0.5">
              <p className="font-mono text-xl font-semibold text-fg">
                {formatTokens((gauge?.month_input_tokens ?? 0) + (gauge?.month_output_tokens ?? 0))}
              </p>
              <p className="text-xs text-fg-muted">
                ≈ {formatCurrency((gauge?.month_cost_usd ?? 0) * 0.92, "EUR")}
              </p>
            </div>
          )}
        </Card>

        <Card className="p-4">
          <div className="flex items-center justify-between text-xs text-fg-subtle">
            <span>Efficacité du cache SQLite</span>
            <Database className="h-3.5 w-3.5 text-status-success" />
          </div>
          {gaugeQuery.isLoading ? (
            <Skeleton className="mt-2 h-7 w-24" />
          ) : (
            <div className="mt-2 space-y-0.5">
              <p className="font-mono text-xl font-semibold text-fg">{cacheRate} %</p>
              <p className="text-xs text-fg-muted">
                {gauge?.total_cache_hits ?? 0} hits / {gauge?.total_calls ?? 0} appels
              </p>
            </div>
          )}
        </Card>
      </div>

      {/* Projets récents */}
      <Card>
        <CardHeader>
          <CardTitle>Projets récents</CardTitle>
          <CardDescription>
            Tous les projets sont sauvegardés localement dans SQLite avec reprise par checkpoint.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {projectsQuery.isLoading && (
            <div className="space-y-2.5">
              <Skeleton className="h-16 w-full" />
              <Skeleton className="h-16 w-full" />
            </div>
          )}

          {!projectsQuery.isLoading && (projectsQuery.data?.length ?? 0) === 0 && (
            <div className="rounded-xl border border-dashed border-border p-8 text-center text-sm text-fg-muted">
              Aucun projet pour le moment. Collez une URL de VOD ci-dessus ou lancez la démo 10 min.
            </div>
          )}

          <div className="space-y-2.5">
            {projectsQuery.data?.map((project) => {
              const meta = STATUS_META[project.status] ?? {
                label: project.status,
                variant: "default" as const,
              };
              const totalTok = project.spent_input_tokens + project.spent_output_tokens;
              return (
                <div
                  key={project.id}
                  className="flex flex-col gap-3 rounded-xl border border-border/70 bg-surface-subtle/35 p-3.5 transition-colors hover:border-border-strong sm:flex-row sm:items-center sm:justify-between"
                >
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate text-sm font-semibold text-fg">
                        {project.name}
                      </span>
                      <Badge variant={meta.variant}>{meta.label}</Badge>
                      <Badge variant="default" className="uppercase text-[10px]">
                        {project.mode}
                      </Badge>
                    </div>
                    <div className="flex flex-wrap items-center gap-3 text-xs text-fg-muted">
                      {project.vod_channel && <span>{project.vod_channel}</span>}
                      <span className="inline-flex items-center gap-1">
                        <Clock className="h-3 w-3" />
                        {formatDuration(project.vod_duration_sec)}
                      </span>
                      <span>{project.candidates_count} candidats</span>
                      <span className="inline-flex items-center gap-1">
                        <Film className="h-3 w-3" />
                        {project.clips_count} clips
                      </span>
                      <span className="font-mono">
                        {formatTokens(totalTok)} / {formatTokens(project.token_budget_limit)} tok
                      </span>
                    </div>
                  </div>

                  <div className="flex shrink-0 items-center gap-2">
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => openProject(project, "analysis")}
                    >
                      <Activity className="h-3.5 w-3.5" />
                      Analyse
                    </Button>
                    <Button
                      size="sm"
                      variant="primary"
                      onClick={() => openProject(project, "timeline")}
                    >
                      <Scissors className="h-3.5 w-3.5" />
                      Timeline
                    </Button>
                    <Button
                      size="icon"
                      variant="ghost"
                      aria-label="Supprimer le projet"
                      onClick={() => void handleDelete(project)}
                    >
                      <Trash2 className="h-4 w-4 text-status-danger" />
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
