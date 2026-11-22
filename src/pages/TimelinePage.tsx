import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  CheckCircle2,
  Flame,
  MessageSquare,
  Scissors,
  Sparkles,
  VolumeX,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { commands, type CandidateRecord } from "@/bindings";
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

export function TimelinePage() {
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);

  const [zoom, setZoom] = useState(1);
  const [selectedCandidateId, setSelectedCandidateId] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  const projectsQuery = useQuery({
    queryKey: ["projects"],
    queryFn: async () => {
      const res = await commands.listProjectsCmd();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const selectedProjectId = activeProjectId ?? projectsQuery.data?.[0]?.id ?? null;

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

  const data = timelineQuery.data;

  const selectedCandidate: CandidateRecord | null = useMemo(() => {
    if (!data?.candidates.length) return null;
    return (
      data.candidates.find((c) => c.id === selectedCandidateId) ?? data.candidates[0]
    );
  }, [data?.candidates, selectedCandidateId]);

  const linkedClip = useMemo(() => {
    if (!data || !selectedCandidate) return null;
    return data.clips.find((cl) => cl.candidate_id === selectedCandidate.id) ?? null;
  }, [data, selectedCandidate]);

  // Dessin Canvas HiDPI haute fluidité : forme d'onde audio, densité du chat, zones mutées hachurées, candidats
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !data || data.signal_windows.length === 0) return;

    const dpr = window.devicePixelRatio || 1;
    const cssWidth = Math.max(820, Math.round(920 * zoom));
    const cssHeight = 230;
    canvas.width = Math.round(cssWidth * dpr);
    canvas.height = Math.round(cssHeight * dpr);
    canvas.style.width = `${cssWidth}px`;
    canvas.style.height = `${cssHeight}px`;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);

    const windows = data.signal_windows;
    const totalDur = Math.max(
      data.vod.duration_sec,
      windows[windows.length - 1]?.end_sec ?? 60,
      1,
    );
    const padTop = 24;
    const padBottom = 26;
    const plotH = cssHeight - padTop - padBottom;
    const toX = (sec: number) => (sec / totalDur) * cssWidth;

    // Fond sombre discret
    ctx.fillStyle = "rgba(13, 15, 24, 0.92)";
    ctx.fillRect(0, 0, cssWidth, cssHeight);

    // Grille temporelle
    ctx.strokeStyle = "rgba(148, 163, 184, 0.14)";
    ctx.fillStyle = "rgba(165, 172, 194, 0.75)";
    ctx.font = "10px monospace";
    const tickStep = totalDur > 7200 ? 900 : totalDur > 1800 ? 300 : 60;
    for (let t = 0; t <= totalDur; t += tickStep) {
      const x = toX(t);
      ctx.beginPath();
      ctx.moveTo(x, padTop);
      ctx.lineTo(x, cssHeight - padBottom);
      ctx.stroke();
      ctx.fillText(formatDuration(t), x + 3, cssHeight - 8);
    }

    // 1. Zones mutées hachurées (interdites au clipping)
    for (const w of windows) {
      if (!w.is_muted) continue;
      const x0 = toX(w.start_sec);
      const x1 = toX(w.end_sec);
      ctx.fillStyle = "rgba(248, 113, 113, 0.16)";
      ctx.fillRect(x0, padTop, Math.max(2, x1 - x0), plotH);
      ctx.strokeStyle = "rgba(248, 113, 113, 0.45)";
      ctx.beginPath();
      for (let hx = x0 - plotH; hx < x1; hx += 10) {
        ctx.moveTo(Math.max(x0, hx), padTop);
        ctx.lineTo(Math.min(x1, hx + plotH), padTop + plotH);
      }
      ctx.stroke();
    }

    // 2. Forme d'onde audio (LUFS + peak normalisé)
    const barW = Math.max(1.5, cssWidth / windows.length - 0.5);
    for (let i = 0; i < windows.length; i += 1) {
      const w = windows[i];
      const normAudio = Math.max(0.04, Math.min(1, (w.audio_lufs + 60) / 55));
      const h = normAudio * (plotH * 0.65);
      const x = toX(w.start_sec);
      const y = padTop + plotH - h;
      ctx.fillStyle = w.is_muted
        ? "rgba(248, 113, 113, 0.25)"
        : "rgba(155, 107, 255, 0.35)";
      ctx.fillRect(x, y, barW, h);
    }

    // 3. Courbe de densité du chat (cyan)
    const maxChat = Math.max(
      1,
      ...windows.map((w) => w.chat_rate + w.emote_score * 0.6),
    );
    ctx.beginPath();
    for (let i = 0; i < windows.length; i += 1) {
      const w = windows[i];
      const x = toX((w.start_sec + w.end_sec) * 0.5);
      const val = (w.chat_rate + w.emote_score * 0.6) / maxChat;
      const y = padTop + plotH - val * (plotH * 0.92);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = "rgb(34, 211, 238)";
    ctx.lineWidth = 2;
    ctx.stroke();

    // 4. Fenêtres & marqueurs de candidats colorés par score
    for (const c of data.candidates) {
      const x0 = toX(c.start_sec);
      const x1 = toX(c.end_sec);
      const xp = toX(c.peak_sec);
      const isSelected = selectedCandidate?.id === c.id;

      const color =
        c.stage_a_keep === true
          ? "rgba(52, 211, 153, 0.28)"
          : c.stage_a_keep === false
            ? "rgba(148, 163, 184, 0.16)"
            : c.local_score >= 0.7
              ? "rgba(34, 211, 238, 0.24)"
              : "rgba(155, 107, 255, 0.22)";

      ctx.fillStyle = color;
      ctx.fillRect(x0, padTop, Math.max(4, x1 - x0), plotH);

      if (isSelected) {
        ctx.strokeStyle = "#ffffff";
        ctx.lineWidth = 2;
        ctx.strokeRect(x0, padTop + 1, Math.max(4, x1 - x0), plotH - 2);
      }

      // Pastille numérotée au sommet du pic
      ctx.fillStyle =
        c.stage_a_keep === true
          ? "#34d399"
          : c.stage_a_keep === false
            ? "#94a3b8"
            : "#a78bfa";
      ctx.beginPath();
      ctx.arc(xp, 12, isSelected ? 8 : 6, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = "#0b0c12";
      ctx.font = "bold 9px sans-serif";
      ctx.textAlign = "center";
      ctx.fillText(String(c.rank_index), xp, 15);
      ctx.textAlign = "start";
    }
  }, [data, zoom, selectedCandidate?.id]);

  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!data || !canvasRef.current || data.candidates.length === 0) return;
    const rect = canvasRef.current.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const totalDur = Math.max(
      data.vod.duration_sec,
      data.signal_windows[data.signal_windows.length - 1]?.end_sec ?? 60,
      1,
    );
    const clickSec = (clickX / rect.width) * totalDur;

    // Sélectionne le candidat contenant le clic ou le plus proche
    let best = data.candidates[0];
    let bestDist = Infinity;
    for (const c of data.candidates) {
      const dist =
        clickSec >= c.start_sec && clickSec <= c.end_sec
          ? 0
          : Math.abs(c.peak_sec - clickSec);
      if (dist < bestDist) {
        bestDist = dist;
        best = c;
      }
    }
    setSelectedCandidateId(best.id);
  };

  if (!selectedProjectId || !data || data.signal_windows.length === 0) {
    return (
      <div className="space-y-6">
        <header className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Timeline</h1>
          <p className="text-sm text-fg-muted">
            Forme d'onde audio, densité du chat, zones mutées hachurées et marqueurs de candidats.
          </p>
        </header>
        <EmptyState
          icon={Scissors}
          title="Aucun signal local calculé pour ce projet"
          description="Lancez d'abord l'analyse (locale ou complète) depuis l'écran Analyse pour générer la Timeline."
          actions={
            <Button variant="primary" onClick={() => setActiveScreen("analysis")}>
              <Activity className="h-4 w-4" />
              Ouvrir l'Analyse
            </Button>
          }
        />
      </div>
    );
  }

  let parsedTranscript = "";
  if (selectedCandidate?.transcript_json) {
    try {
      const tr = JSON.parse(selectedCandidate.transcript_json) as { compact_text?: string };
      parsedTranscript = tr.compact_text ?? "";
    } catch {
      parsedTranscript = "";
    }
  }

  let parsedChat: { r?: number; e?: string[]; m?: string[] } = {};
  if (selectedCandidate?.chat_summary_json) {
    try {
      parsedChat = JSON.parse(selectedCandidate.chat_summary_json);
    } catch {
      parsedChat = {};
    }
  }

  return (
    <div className="space-y-6">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Timeline</h1>
          <p className="text-sm text-fg-muted">
            {data.project.name} · {data.signal_windows.length} fenêtres 5 s · {data.candidates.length} candidats · {data.clips.length} clips retenus
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {projectsQuery.data && projectsQuery.data.length > 1 && (
            <select
              value={selectedProjectId}
              onChange={(e) => {
                setActiveProjectId(e.target.value);
                setSelectedCandidateId(null);
              }}
              className="h-8 rounded-xl border border-border bg-surface-elevated px-2.5 text-xs text-fg"
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
            onClick={() => setZoom((z) => Math.max(1, Number((z / 1.5).toFixed(2))))}
          >
            <ZoomOut className="h-3.5 w-3.5" />
          </Button>
          <span className="font-mono text-xs text-fg-muted">{zoom.toFixed(1)}×</span>
          <Button
            size="sm"
            variant="outline"
            onClick={() => setZoom((z) => Math.min(16, Number((z * 1.5).toFixed(2))))}
          >
            <ZoomIn className="h-3.5 w-3.5" />
          </Button>
        </div>
      </header>

      {/* Canvas Timeline scrollable + légende */}
      <Card className="p-4 space-y-3">
        <div className="flex flex-wrap items-center gap-4 text-xs text-fg-muted">
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-full bg-accent-violet" /> Forme d'onde audio
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-full bg-accent-cyan" /> Densité chat & emotes
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-full bg-status-success" /> Retenu Gemini
          </span>
          <span className="inline-flex items-center gap-1.5">
            <VolumeX className="h-3.5 w-3.5 text-status-danger" /> Zones mutées (exclues)
          </span>
        </div>

        <div className="overflow-x-auto rounded-xl border border-border/70">
          <canvas
            ref={canvasRef}
            onClick={handleCanvasClick}
            className="block cursor-pointer"
          />
        </div>
      </Card>

      {/* Liste des candidats + inspecteur détaillé */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Card className="p-4 space-y-2 lg:col-span-1">
          <h2 className="text-sm font-semibold text-fg">Candidats ({data.candidates.length})</h2>
          <div className="max-h-80 space-y-1.5 overflow-y-auto pr-1">
            {data.candidates.map((c) => {
              const active = selectedCandidate?.id === c.id;
              return (
                <button
                  key={c.id}
                  type="button"
                  onClick={() => setSelectedCandidateId(c.id)}
                  className={`flex w-full items-center justify-between rounded-xl border px-3 py-2 text-left text-xs transition-colors ${
                    active
                      ? "border-accent-cyan bg-accent-cyan/15 text-fg"
                      : "border-border/60 bg-surface-subtle/35 text-fg-muted hover:text-fg"
                  }`}
                >
                  <div className="min-w-0">
                    <span className="font-semibold text-fg">#{c.rank_index}</span>{" "}
                    <span className="font-mono">
                      {formatDuration(c.start_sec)} – {formatDuration(c.end_sec)}
                    </span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    {c.stage_a_score != null ? (
                      <Badge variant={c.stage_a_keep ? "success" : "default"}>
                        {c.stage_a_score}/100
                      </Badge>
                    ) : (
                      <Badge variant="violet">{Math.round(c.local_score * 100)}%</Badge>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        </Card>

        {selectedCandidate && (
          <Card className="lg:col-span-2">
            <CardHeader>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <CardTitle>
                  Candidat #{selectedCandidate.rank_index} ·{" "}
                  {formatDuration(selectedCandidate.start_sec)} →{" "}
                  {formatDuration(selectedCandidate.end_sec)}
                </CardTitle>
                <div className="flex flex-wrap gap-1.5">
                  <Badge variant="violet">
                    <Flame className="h-3 w-3" />
                    Score local {Math.round(selectedCandidate.local_score * 100)} %
                  </Badge>
                  {selectedCandidate.stage_a_category && (
                    <Badge variant="cyan">{selectedCandidate.stage_a_category}</Badge>
                  )}
                  {selectedCandidate.stage_a_keep === true && (
                    <Badge variant="success">
                      <CheckCircle2 className="h-3 w-3" />
                      Retenu ({selectedCandidate.stage_a_score}/100)
                    </Badge>
                  )}
                </div>
              </div>
              {selectedCandidate.stage_a_reason && (
                <CardDescription>
                  Verdict Gemini Étage A : « {selectedCandidate.stage_a_reason} »
                </CardDescription>
              )}
            </CardHeader>

            <CardContent className="space-y-4 text-xs">
              {linkedClip && (
                <div className="rounded-xl border border-accent-violet/40 bg-accent-violet/10 p-3.5 space-y-1.5">
                  <div className="flex items-center gap-1.5 font-semibold text-fg">
                    <Sparkles className="h-3.5 w-3.5 text-accent-cyan" />
                    Habillage Gemini Étage B
                  </div>
                  {linkedClip.hook && (
                    <p>
                      <span className="text-fg-subtle">Accroche :</span>{" "}
                      <span className="font-semibold text-fg">{linkedClip.hook}</span>
                    </p>
                  )}
                  <p>
                    <span className="text-fg-subtle">Titre :</span>{" "}
                    <span className="text-fg">{linkedClip.title}</span>
                  </p>
                  {linkedClip.caption && (
                    <p className="text-fg-muted">{linkedClip.caption}</p>
                  )}
                  {linkedClip.hashtags.length > 0 && (
                    <div className="flex flex-wrap gap-1 pt-1">
                      {linkedClip.hashtags.map((tag) => (
                        <Badge key={tag} variant="default">
                          #{tag}
                        </Badge>
                      ))}
                    </div>
                  )}
                </div>
              )}

              <div className="space-y-1.5">
                <p className="flex items-center gap-1.5 font-semibold text-fg">
                  <MessageSquare className="h-3.5 w-3.5 text-accent-cyan" />
                  Résumé chat compact ({parsedChat.r ?? 0} msg/s)
                </p>
                {parsedChat.e && parsedChat.e.length > 0 && (
                  <div className="flex flex-wrap gap-1.5">
                    {parsedChat.e.map((em) => (
                      <Badge key={em} variant="cyan">
                        {em}
                      </Badge>
                    ))}
                  </div>
                )}
                {parsedChat.m && parsedChat.m.length > 0 && (
                  <ul className="list-disc pl-5 text-fg-muted space-y-0.5">
                    {parsedChat.m.map((m, i) => (
                      <li key={i}>{m}</li>
                    ))}
                  </ul>
                )}
              </div>

              <div className="space-y-1">
                <p className="font-semibold text-fg">Transcription locale (whisper.cpp)</p>
                <div className="max-h-36 overflow-y-auto rounded-xl border border-border/60 bg-surface-base/70 p-3 font-mono text-[11px] leading-relaxed text-fg-muted">
                  {parsedTranscript || "Non encore transcrite."}
                </div>
              </div>
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  );
}
