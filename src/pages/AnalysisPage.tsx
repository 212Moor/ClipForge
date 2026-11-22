import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  AlertTriangle,
  BarChart3,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Cpu,
  Pause,
  Play,
  RefreshCw,
  Scissors,
  Terminal,
} from "lucide-react";
import {
  commands,
  events,
  type PipelineLogEvent,
  type ProgressEvent,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { formatCurrency, formatTokens } from "@/lib/format";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

const STEPS = [
  { id: "ingest", label: "1. Ingestion (audio + chat + zones mutées)" },
  { id: "signals", label: "2. Signaux locaux (pas de 5 s, 0 token)" },
  { id: "scoring", label: "3. Scoring médiane/MAD & candidats" },
  { id: "asr", label: "4. Transcription locale whisper.cpp (+15 s)" },
  { id: "stage_a", label: "5. Tri éditorial Gemini Étage A" },
  { id: "stage_b", label: "6. Finition Gemini Étage B (top K)" },
];

export function AnalysisPage() {
  const queryClient = useQueryClient();
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);

  const [liveProgress, setLiveProgress] = useState<ProgressEvent | null>(null);
  const [logs, setLogs] = useState<PipelineLogEvent[]>([]);
  const [logsOpen, setLogsOpen] = useState(true);
  const [newBudgetInput, setNewBudgetInput] = useState(250_000);

  const projectsQuery = useQuery({
    queryKey: ["projects"],
    queryFn: async () => {
      const res = await commands.listProjectsCmd();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const selectedProjectId = activeProjectId ?? projectsQuery.data?.[0]?.id ?? null;

  useEffect(() => {
    if (!activeProjectId && projectsQuery.data?.[0]?.id) {
      setActiveProjectId(projectsQuery.data[0].id);
    }
  }, [activeProjectId, projectsQuery.data, setActiveProjectId]);

  const statusQuery = useQuery({
    queryKey: ["pipeline-status", selectedProjectId],
    enabled: Boolean(selectedProjectId),
    queryFn: async () => {
      if (!selectedProjectId) throw new Error("Aucun projet");
      const res = await commands.getProjectPipelineStatus(selectedProjectId);
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  // Événements poussés en direct depuis Rust (zéro polling)
  useEffect(() => {
    const unlistenProg = events.progressEvent.listen((ev) => {
      if (ev.payload.scope !== "pipeline") return;
      if (selectedProjectId && ev.payload.job_id !== selectedProjectId) return;
      setLiveProgress(ev.payload);
      if (
        ["completed", "paused", "failed", "budget_paused", "scoring", "stage_a", "stage_b"].includes(
          ev.payload.step,
        )
      ) {
        void queryClient.invalidateQueries({ queryKey: ["pipeline-status", selectedProjectId] });
        void queryClient.invalidateQueries({ queryKey: ["projects"] });
        void queryClient.invalidateQueries({ queryKey: ["token-gauge"] });
      }
    });

    const unlistenLogs = events.pipelineLogEvent.listen((ev) => {
      if (selectedProjectId && ev.payload.project_id !== selectedProjectId) return;
      setLogs((prev) => [...prev.slice(-149), ev.payload]);
    });

    return () => {
      void unlistenProg.then((u) => u());
      void unlistenLogs.then((u) => u());
    };
  }, [selectedProjectId, queryClient]);

  if (!selectedProjectId) {
    return (
      <div className="space-y-6">
        <header className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Analyse</h1>
          <p className="text-sm text-fg-muted">Suivi du pipeline local puis Gemini avec reprise par checkpoint.</p>
        </header>
        <EmptyState
          icon={Cpu}
          title="Aucun projet sélectionné"
          description="Créez un nouveau projet ou lancez la démo 10 min depuis l'Accueil."
          actions={
            <Button variant="primary" onClick={() => setActiveScreen("new-project")}>
              Nouveau projet
            </Button>
          }
        />
      </div>
    );
  }

  const project = projectsQuery.data?.find((p) => p.id === selectedProjectId);
  const snap = statusQuery.data;
  const cp = snap?.checkpoint;
  const jobState = snap?.job?.state ?? "queued";
  const isRunning = jobState === "running";
  const pct = liveProgress?.percent ?? snap?.job?.progress ?? 0;

  const isStepDone = (id: string): boolean => {
    if (!cp) return false;
    switch (id) {
      case "ingest":
        return cp.ingest_done;
      case "signals":
        return cp.signals_done;
      case "scoring":
        return cp.scoring_done;
      case "asr":
        return cp.total_candidates > 0 && cp.asr_completed_ids.length >= cp.total_candidates;
      case "stage_a":
        return cp.stage_a_completed_batches.length > 0 && (cp.current_step === "stage_b" || cp.current_step === "completed");
      case "stage_b":
        return cp.stage_b_done || cp.current_step === "completed";
      default:
        return false;
    }
  };

  const handleStartOrResume = async (localOnly: boolean) => {
    const res = await commands.startProjectAnalysis(selectedProjectId, localOnly);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success("Pipeline démarré / repris depuis le dernier checkpoint.");
    await queryClient.invalidateQueries({ queryKey: ["pipeline-status", selectedProjectId] });
  };

  const handlePause = async () => {
    const res = await commands.pauseProjectAnalysis(selectedProjectId);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.info("Pause demandée — le checkpoint courant sera conservé.");
  };

  const handleRaiseBudgetAndResume = async () => {
    const res = await commands.updateProjectBudgetLimit(selectedProjectId, newBudgetInput);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    await handleStartOrResume(false);
  };

  const usedTokens = (snap?.spent_input_tokens ?? 0) + (snap?.spent_output_tokens ?? 0);
  const budgetLimit = snap?.budget_limit_tokens ?? project?.token_budget_limit ?? 150_000;
  const budgetPct = Math.min(100, Math.round((usedTokens / Math.max(1, budgetLimit)) * 100));

  return (
    <div className="space-y-6">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Analyse</h1>
          <p className="text-sm text-fg-muted">
            {project?.name ?? "Projet actif"} — reprise exacte par checkpoint SQLite après pause ou coupure réseau.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {projectsQuery.data && projectsQuery.data.length > 1 && (
            <select
              value={selectedProjectId}
              onChange={(e) => {
                setActiveProjectId(e.target.value);
                setLiveProgress(null);
              }}
              className="h-9 rounded-xl border border-border bg-surface-elevated px-3 text-xs text-fg"
            >
              {projectsQuery.data.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          )}

          {isRunning ? (
            <Button variant="secondary" size="sm" onClick={() => void handlePause()}>
              <Pause className="h-3.5 w-3.5" />
              Mettre en pause
            </Button>
          ) : (
            <>
              <Button variant="secondary" size="sm" onClick={() => void handleStartOrResume(true)}>
                <Cpu className="h-3.5 w-3.5" />
                Local seul (0 tok)
              </Button>
              <Button variant="primary" size="sm" onClick={() => void handleStartOrResume(false)}>
                <Play className="h-3.5 w-3.5" />
                {snap?.job ? "Reprendre / Relancer" : "Lancer (Local + Gemini)"}
              </Button>
            </>
          )}
        </div>
      </header>

      {/* Barre de progression + compteur de tokens en direct */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Card className="p-5 lg:col-span-2">
          <div className="flex items-center justify-between text-sm">
            <span className="font-medium text-fg">
              {liveProgress?.message ?? snap?.job?.error_message ?? "Prêt à lancer ou reprendre"}
            </span>
            <span className="font-mono text-accent-cyan">{Math.round(pct)} %</span>
          </div>
          <div className="mt-3 h-2 w-full overflow-hidden rounded-full bg-surface-subtle">
            <div
              className="h-full rounded-full bg-gradient-to-r from-accent-violet to-accent-cyan transition-[width] duration-200"
              style={{ width: `${Math.min(100, Math.max(0, pct))}%` }}
            />
          </div>

          <div className="mt-5 grid grid-cols-1 gap-2 sm:grid-cols-2">
            {STEPS.map((step) => {
              const done = isStepDone(step.id);
              const current =
                (liveProgress?.step ?? cp?.current_step) === step.id && isRunning;
              return (
                <div
                  key={step.id}
                  className={`flex items-center gap-2.5 rounded-xl border px-3 py-2 text-xs ${
                    done
                      ? "border-status-success/40 bg-status-success/10 text-fg"
                      : current
                        ? "border-accent-cyan/50 bg-accent-cyan/10 text-fg"
                        : "border-border/60 bg-surface-subtle/30 text-fg-muted"
                  }`}
                >
                  {done ? (
                    <CheckCircle2 className="h-4 w-4 shrink-0 text-status-success" />
                  ) : current ? (
                    <RefreshCw className="h-4 w-4 shrink-0 animate-spin text-accent-cyan" />
                  ) : (
                    <span className="h-2.5 w-2.5 shrink-0 rounded-full bg-border-strong" />
                  )}
                  <span className="truncate">{step.label}</span>
                </div>
              );
            })}
          </div>
        </Card>

        <Card className="p-5 space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold text-fg">Compteur tokens en direct</h2>
            <Badge variant={budgetPct >= 90 ? "warning" : "cyan"}>{budgetPct} %</Badge>
          </div>
          <div className="space-y-1">
            <p className="font-mono text-2xl font-semibold text-fg">
              {formatTokens(usedTokens)}{" "}
              <span className="text-xs font-normal text-fg-subtle">
                / {formatTokens(budgetLimit)}
              </span>
            </p>
            <p className="text-xs text-fg-muted">
              Entrée : {formatTokens(snap?.spent_input_tokens ?? 0)} · Sortie :{" "}
              {formatTokens(snap?.spent_output_tokens ?? 0)}
            </p>
            <p className="text-xs text-accent-cyan">
              Coût réel : {formatCurrency((snap?.spent_cost_usd ?? 0) * 0.92, "EUR")}
            </p>
          </div>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-subtle">
            <div
              className="h-full rounded-full bg-accent-violet transition-[width] duration-200"
              style={{ width: `${budgetPct}%` }}
            />
          </div>

          <div className="flex flex-col gap-2 pt-2">
            <Button variant="primary" size="sm" onClick={() => setActiveScreen("timeline")}>
              <Scissors className="h-3.5 w-3.5" />
              Ouvrir la Timeline
            </Button>
            <Button variant="outline" size="sm" onClick={() => setActiveScreen("costs")}>
              <BarChart3 className="h-3.5 w-3.5" />
              Détail Coûts & cache
            </Button>
          </div>
        </Card>
      </div>

      {/* Alerte plafond dur atteint -> augmentation + reprise propre */}
      {project?.status === "budget_paused" && (
        <Card className="border-status-warning/50 bg-status-warning/10 p-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-start gap-2.5 text-sm">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-status-warning" />
              <div>
                <p className="font-semibold text-fg">Plafond de tokens atteint — traitement arrêté proprement</p>
                <p className="text-xs text-fg-muted">{snap?.job?.error_message}</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <input
                type="number"
                step={25000}
                value={newBudgetInput}
                onChange={(e) => setNewBudgetInput(Number(e.target.value))}
                className="h-9 w-32 rounded-xl border border-border bg-surface-base px-3 font-mono text-xs text-fg"
              />
              <Button size="sm" variant="primary" onClick={() => void handleRaiseBudgetAndResume()}>
                Augmenter & reprendre
              </Button>
            </div>
          </div>
        </Card>
      )}

      {/* Journal repliable temps réel */}
      <Card>
        <CardHeader className="pb-2">
          <button
            type="button"
            onClick={() => setLogsOpen((v) => !v)}
            className="flex w-full items-center justify-between text-left"
          >
            <div className="flex items-center gap-2">
              <Terminal className="h-4 w-4 text-accent-cyan" />
              <CardTitle>Journal d'exécution ({logs.length})</CardTitle>
            </div>
            {logsOpen ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
          </button>
          <CardDescription>Événements poussés en direct depuis Rust.</CardDescription>
        </CardHeader>
        {logsOpen && (
          <CardContent>
            <div className="max-h-56 overflow-y-auto rounded-xl border border-border/60 bg-surface-base/75 p-3 font-mono text-xs space-y-1">
              {logs.length === 0 ? (
                <p className="text-fg-subtle">En attente d'événements du pipeline…</p>
              ) : (
                logs.map((entry, idx) => (
                  <div key={idx} className="flex gap-2">
                    <span className="shrink-0 text-fg-subtle">[{entry.timestamp}]</span>
                    <span
                      className={
                        entry.level === "error"
                          ? "text-status-danger"
                          : entry.level === "warn"
                            ? "text-status-warning"
                            : "text-fg"
                      }
                    >
                      {entry.message}
                    </span>
                  </div>
                ))
              )}
            </div>
          </CardContent>
        )}
      </Card>
    </div>
  );
}
