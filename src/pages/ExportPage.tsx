import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  CheckCircle2,
  Copy,
  Cpu,
  Download,
  FileSpreadsheet,
  FolderOpen,
  Play,
  RefreshCw,
  Square,
} from "lucide-react";
import {
  commands,
  events,
  type ClipRecord,
  type ProgressEvent,
  type RenderSettings,
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
import { formatDuration } from "@/lib/format";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

export function ExportPage() {
  const queryClient = useQueryClient();
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);

  const [progressByClip, setProgressByClip] = useState<Record<string, ProgressEvent>>({});
  const [settingsDraft, setSettingsDraft] = useState<RenderSettings | null>(null);

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

  const timelineQuery = useQuery({
    queryKey: ["project-timeline", selectedProjectId],
    enabled: Boolean(selectedProjectId),
    queryFn: async () => {
      if (!selectedProjectId) throw new Error("Aucun projet");
      const res = await commands.getProjectTimeline(selectedProjectId);
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const encodersQuery = useQuery({
    queryKey: ["render-encoders"],
    queryFn: async () => {
      const res = await commands.detectRenderEncoders();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const settingsQuery = useQuery({
    queryKey: ["render-settings"],
    queryFn: async () => {
      const res = await commands.getRenderSettings();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  useEffect(() => {
    if (settingsQuery.data) {
      setSettingsDraft(settingsQuery.data);
    }
  }, [settingsQuery.data]);

  // Progression FFmpeg poussée en direct depuis Rust (aucun polling)
  useEffect(() => {
    const unlistenPromise = events.progressEvent.listen((ev) => {
      if (ev.payload.scope !== "render") return;
      setProgressByClip((prev) => ({
        ...prev,
        [ev.payload.job_id]: ev.payload,
      }));
      if (["done", "failed", "cancelled"].includes(ev.payload.step)) {
        void queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      }
    });
    return () => {
      void unlistenPromise.then((u) => u());
    };
  }, [selectedProjectId, queryClient]);

  const handleSaveRenderSettings = async (next: RenderSettings) => {
    setSettingsDraft(next);
    const res = await commands.setRenderSettings(next);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success("Paramètres d'encodage enregistrés.");
  };

  const handleRenderOne = async (clipId: string) => {
    const res = await commands.enqueueClipRender(clipId);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.info("Clip ajouté à la file de rendu 9:16.");
  };

  const handleRenderAllApproved = async () => {
    if (!selectedProjectId) return;
    const res = await commands.enqueueProjectApprovedRenders(selectedProjectId);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    if (res.data === 0) {
      toast.warning("Aucun clip validé à exporter. Validez d'abord vos clips dans Revue.");
      return;
    }
    toast.success(`${res.data} clip(s) validé(s) envoyé(s) en file de rendu.`);
  };

  const handleCancelRender = async (clipId: string) => {
    const res = await commands.cancelClipRender(clipId);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.info("Annulation demandée.");
  };

  const handleCopyCaption = async (clip: ClipRecord) => {
    const tags = clip.hashtags.map((t) => `#${t.replace(/^#/, "")}`).join(" ");
    const fullText = [clip.title, clip.caption, tags].filter(Boolean).join("\n\n");
    try {
      await navigator.clipboard.writeText(fullText);
      toast.success("Légende et hashtags copiés dans le presse-papiers !");
    } catch {
      toast.error("Impossible d'accéder au presse-papiers.");
    }
  };

  const handleOpenPath = async (path: string) => {
    const res = await commands.revealInExplorer(path);
    if (res.status === "error") {
      toast.error(res.error.message);
    }
  };

  const handleExportCsv = async () => {
    if (!selectedProjectId) return;
    const res = await commands.exportProjectCsv(selectedProjectId);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success("CSV récapitulatif généré.");
    await handleOpenPath(res.data);
  };

  const clips = timelineQuery.data?.clips ?? [];
  const approvedCount = clips.filter((c) => c.review_status === "approved").length;
  const doneCount = clips.filter((c) => c.render_status === "done").length;
  const enc = encodersQuery.data;

  return (
    <div className="space-y-6">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">File d'export</h1>
          <p className="text-sm text-fg-muted">
            Rendu 1080×1920 H.264 / AAC 192 kb/s / loudnorm −14 LUFS + sous-titres ASS mot à mot + fichier légende .txt & CSV.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {projectsQuery.data && projectsQuery.data.length > 1 && (
            <select
              value={selectedProjectId ?? ""}
              onChange={(e) => setActiveProjectId(e.target.value)}
              className="h-9 rounded-xl border border-border bg-surface-elevated px-3 text-xs text-fg"
            >
              {projectsQuery.data.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={!selectedProjectId || clips.length === 0}
            onClick={() => void handleExportCsv()}
          >
            <FileSpreadsheet className="h-3.5 w-3.5" />
            Exporter CSV
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={!selectedProjectId || approvedCount === 0}
            onClick={() => void handleRenderAllApproved()}
          >
            <Download className="h-3.5 w-3.5" />
            Exporter les validés ({approvedCount})
          </Button>
        </div>
      </header>

      {/* Carte encodeur matériel & réglages de rendu */}
      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <Cpu className="h-4 w-4 text-accent-cyan" />
              <CardTitle>Encodeur matériel & profil d'export</CardTitle>
            </div>
            {enc && (
              <div className="flex flex-wrap gap-1.5">
                {enc.nvenc && <Badge variant="success">NVENC</Badge>}
                {enc.amf && <Badge variant="cyan">AMF</Badge>}
                {enc.qsv && <Badge variant="violet">QSV</Badge>}
                <Badge variant="default">Recommandé : {enc.recommended}</Badge>
              </div>
            )}
          </div>
          <CardDescription>
            Télécharge uniquement les segments vidéo des clips retenus (±5 s), jamais la VOD complète.
          </CardDescription>
        </CardHeader>
        {settingsDraft && (
          <CardContent className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <div className="space-y-1">
              <label className="text-xs text-fg-muted">Encodeur H.264</label>
              <select
                value={settingsDraft.encoder}
                onChange={(e) =>
                  void handleSaveRenderSettings({ ...settingsDraft, encoder: e.target.value })
                }
                className="h-9 w-full rounded-xl border border-border bg-surface-elevated px-3 text-xs text-fg"
              >
                <option value="auto">Automatique ({enc?.recommended ?? "détection"})</option>
                <option value="h264_nvenc">NVIDIA NVENC (h264_nvenc)</option>
                <option value="h264_amf">AMD AMF (h264_amf)</option>
                <option value="h264_qsv">Intel QuickSync (h264_qsv)</option>
                <option value="libx264">CPU universel (libx264)</option>
              </select>
            </div>
            <div className="space-y-1">
              <label className="text-xs text-fg-muted">Fréquence d'images</label>
              <select
                value={settingsDraft.fps}
                onChange={(e) =>
                  void handleSaveRenderSettings({
                    ...settingsDraft,
                    fps: Number(e.target.value),
                  })
                }
                className="h-9 w-full rounded-xl border border-border bg-surface-elevated px-3 text-xs text-fg"
              >
                <option value={30}>30 fps (standard TikTok)</option>
                <option value={60}>60 fps (fluide gaming)</option>
              </select>
            </div>
            <div className="space-y-1">
              <label className="text-xs text-fg-muted">Jobs en parallèle</label>
              <select
                value={settingsDraft.max_parallel}
                onChange={(e) =>
                  void handleSaveRenderSettings({
                    ...settingsDraft,
                    max_parallel: Number(e.target.value),
                  })
                }
                className="h-9 w-full rounded-xl border border-border bg-surface-elevated px-3 text-xs text-fg"
              >
                <option value={1}>1 job (CPU / GPU modeste)</option>
                <option value={2}>2 jobs (GPU dédié recommandé)</option>
              </select>
            </div>
          </CardContent>
        )}
      </Card>

      {/* Liste des clips dans la file d'export */}
      {clips.length === 0 ? (
        <EmptyState
          icon={Download}
          title="Aucun clip dans ce projet"
          description="Passez par l'écran Revue des clips pour valider les extraits que vous souhaitez exporter."
          actions={
            <Button variant="primary" onClick={() => setActiveScreen("review")}>
              Ouvrir la Revue des clips
            </Button>
          }
        />
      ) : (
        <div className="space-y-3">
          <div className="flex items-center justify-between text-xs text-fg-muted">
            <span>
              {doneCount} / {clips.length} clip(s) exporté(s)
            </span>
          </div>

          {clips.map((clip) => {
            const prog = progressByClip[clip.id];
            const isRendering =
              clip.render_status === "rendering" ||
              clip.render_status === "queued" ||
              (prog != null && !["done", "failed", "cancelled"].includes(prog.step));

            return (
              <Card key={clip.id} className="p-4 space-y-3">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate text-sm font-semibold text-fg">
                        {clip.title || "Clip sans titre"}
                      </span>
                      <Badge variant="default" className="font-mono text-[10px]">
                        {formatDuration(clip.start_sec)} – {formatDuration(clip.end_sec)}
                      </Badge>
                      <Badge variant="violet" className="uppercase text-[10px]">
                        {clip.layout_mode}
                      </Badge>
                      <Badge
                        variant={
                          clip.review_status === "approved"
                            ? "success"
                            : clip.review_status === "rejected"
                              ? "warning"
                              : "default"
                        }
                      >
                        {clip.review_status === "approved"
                          ? "Validé"
                          : clip.review_status === "rejected"
                            ? "Rejeté"
                            : "À relire"}
                      </Badge>
                      {clip.render_status === "done" && (
                        <Badge variant="success">
                          <CheckCircle2 className="h-3 w-3" />
                          Exporté
                        </Badge>
                      )}
                      {clip.render_status === "failed" && (
                        <Badge variant="warning">Échec</Badge>
                      )}
                    </div>
                    {clip.output_mp4_path && (
                      <p className="truncate font-mono text-[11px] text-fg-subtle">
                        {clip.output_mp4_path}
                      </p>
                    )}
                  </div>

                  <div className="flex flex-wrap shrink-0 items-center gap-2">
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void handleCopyCaption(clip)}
                    >
                      <Copy className="h-3.5 w-3.5" />
                      Copier légende
                    </Button>

                    {clip.output_mp4_path && (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => void handleOpenPath(clip.output_mp4_path!)}
                      >
                        <FolderOpen className="h-3.5 w-3.5" />
                        Ouvrir le dossier
                      </Button>
                    )}

                    {isRendering ? (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => void handleCancelRender(clip.id)}
                      >
                        <Square className="h-3.5 w-3.5 text-status-danger" />
                        Annuler
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        variant={clip.render_status === "done" ? "outline" : "primary"}
                        onClick={() => void handleRenderOne(clip.id)}
                      >
                        {clip.render_status === "done" ? (
                          <>
                            <RefreshCw className="h-3.5 w-3.5" />
                            Ré-exporter
                          </>
                        ) : (
                          <>
                            <Play className="h-3.5 w-3.5" />
                            Exporter MP4
                          </>
                        )}
                      </Button>
                    )}
                  </div>
                </div>

                {(isRendering || prog?.step === "failed") && prog && (
                  <div className="space-y-1">
                    <div className="flex items-center justify-between text-xs text-fg-muted">
                      <span>{prog.message}</span>
                      <span className="font-mono">{Math.round(prog.percent)} %</span>
                    </div>
                    <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-subtle">
                      <div
                        className={`h-full rounded-full transition-[width] duration-150 ${
                          prog.step === "failed"
                            ? "bg-status-danger"
                            : "bg-gradient-to-r from-accent-violet to-accent-cyan"
                        }`}
                        style={{ width: `${Math.min(100, Math.max(0, prog.percent))}%` }}
                      />
                    </div>
                  </div>
                )}
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
