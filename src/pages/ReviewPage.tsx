import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { toast } from "sonner";
import {
  Check,
  Download,
  Edit3,
  Film,
  Layout,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Save,
  Shield,
  Sparkles,
  Trash2,
  Type as TypeIcon,
  Wand2,
  X,
} from "lucide-react";
import {
  commands,
  type ClipRecord,
  type FacecamRect,
  type SubtitleStylePreset,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { formatDuration } from "@/lib/format";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

type ReviewFilter = "all" | "pending" | "approved" | "rejected";

interface EditableSubtitleCue { start_sec: number; end_sec: number; text: string; }

interface ClipOverrides {
  cues: EditableSubtitleCue[];
  pan_x: number;
  font_name: string | null;
  primary_color_hex: string | null;
  highlight_color_hex: string | null;
  font_size: number | null;
}

const DEFAULT_FACECAM: FacecamRect = { x: 0.02, y: 0.62, w: 0.24, h: 0.34 };

function parseClipOverrides(clip: ClipRecord): ClipOverrides {
  const base: ClipOverrides = {
    cues: [],
    pan_x: 0,
    font_name: null,
    primary_color_hex: null,
    highlight_color_hex: null,
    font_size: null,
  };
  if (clip.subtitles_override_json) {
    try {
      const parsed = JSON.parse(clip.subtitles_override_json) as Partial<ClipOverrides>;
      return {
        cues: Array.isArray(parsed.cues) ? parsed.cues : [],
        pan_x: typeof parsed.pan_x === "number" ? parsed.pan_x : 0,
        font_name: parsed.font_name ?? null,
        primary_color_hex: parsed.primary_color_hex ?? null,
        highlight_color_hex: parsed.highlight_color_hex ?? null,
        font_size: parsed.font_size ?? null,
      };
    } catch {
      // ignore invalid JSON
    }
  }
  // Initialise les cues éditables depuis la transcription Whisper si aucun override n'existe
  if (clip.transcript_json) {
    try {
      const tr = JSON.parse(clip.transcript_json) as {
        segments?: { start_sec: number; end_sec: number; text: string }[];
      };
      if (Array.isArray(tr.segments)) {
        base.cues = tr.segments
          .filter((s) => s.end_sec > clip.start_sec && s.start_sec < clip.end_sec)
          .map((s) => ({
            start_sec: Number(Math.max(clip.start_sec, s.start_sec).toFixed(2)),
            end_sec: Number(Math.min(clip.end_sec, s.end_sec).toFixed(2)),
            text: s.text.trim(),
          }))
          .filter((c) => c.text.length > 0);
      }
    } catch {
      // ignore
    }
  }
  return base;
}

export function ReviewPage() {
  const queryClient = useQueryClient();
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);

  const [filter, setFilter] = useState<ReviewFilter>("all");
  const [selectedClipId, setSelectedClipId] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [showSafeZones, setShowSafeZones] = useState(true);
  const [promoting, setPromoting] = useState(false);

  // État du tiroir d'édition
  const [editStart, setEditStart] = useState(0);
  const [editEnd, setEditEnd] = useState(30);
  const [editHook, setEditHook] = useState("");
  const [editTitle, setEditTitle] = useState("");
  const [editCaption, setEditCaption] = useState("");
  const [editTags, setEditTags] = useState("");
  const [editCategory, setEditCategory] = useState("fun");
  const [editLayout, setEditLayout] = useState("split");
  const [editStyle, setEditStyle] = useState("bold_pop");
  const [editOverrides, setEditOverrides] = useState<ClipOverrides>({
    cues: [],
    pan_x: 0,
    font_name: null,
    primary_color_hex: null,
    highlight_color_hex: null,
    font_size: null,
  });
  const [facecam, setFacecam] = useState<FacecamRect>(DEFAULT_FACECAM);
  const [saving, setSaving] = useState(false);
  const [regenerating, setRegenerating] = useState(false);

  // Lecteur en boucle
  const mediaRef = useRef<HTMLAudioElement | null>(null);
  const [isLoopPlaying, setIsLoopPlaying] = useState(false);
  const [playheadSec, setPlayheadSec] = useState(0);

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

  const presetsQuery = useQuery({
    queryKey: ["subtitle-presets"],
    queryFn: () => commands.listSubtitlePresets(),
  });

  const renderSettingsQuery = useQuery({
    queryKey: ["render-settings"],
    queryFn: async () => {
      const res = await commands.getRenderSettings();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  useEffect(() => {
    if (renderSettingsQuery.data?.default_facecam) {
      setFacecam(renderSettingsQuery.data.default_facecam);
    }
  }, [renderSettingsQuery.data]);

  const allClips = timelineQuery.data?.clips ?? [];
  const filteredClips = useMemo(() => {
    if (filter === "all") return allClips;
    return allClips.filter((c) => c.review_status === filter);
  }, [allClips, filter]);

  const selectedClip = useMemo(() => {
    if (filteredClips.length === 0) return null;
    return filteredClips.find((c) => c.id === selectedClipId) ?? filteredClips[0];
  }, [filteredClips, selectedClipId]);

  useEffect(() => {
    if (selectedClip && selectedClip.id !== selectedClipId) {
      setSelectedClipId(selectedClip.id);
    }
  }, [selectedClip, selectedClipId]);

  const loadClipIntoDrawer = (clip: ClipRecord) => {
    setEditStart(Number(clip.start_sec.toFixed(1)));
    setEditEnd(Number(clip.end_sec.toFixed(1)));
    setEditHook(clip.hook);
    setEditTitle(clip.title);
    setEditCaption(clip.caption);
    setEditTags(clip.hashtags.join(", "));
    setEditCategory(clip.category);
    setEditLayout(clip.layout_mode);
    setEditStyle(clip.subtitle_style);
    setEditOverrides(parseClipOverrides(clip));
    setPlayheadSec(clip.start_sec);
    setIsLoopPlaying(false);
    setDrawerOpen(true);
  };

  const handleReviewStatus = async (clip: ClipRecord, status: "approved" | "rejected") => {
    const res = await commands.setClipReviewStatus(clip.id, status);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success(status === "approved" ? "Clip validé pour l'export" : "Clip rejeté");
    await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
    await queryClient.invalidateQueries({ queryKey: ["projects"] });

    // Avance automatiquement au clip suivant dans la liste
    const idx = filteredClips.findIndex((c) => c.id === clip.id);
    if (idx >= 0 && idx + 1 < filteredClips.length) {
      setSelectedClipId(filteredClips[idx + 1].id);
    }
  };

  // Raccourcis clavier : ← rejeter, → valider, E éditer, ↑/↓ naviguer, Esc fermer tiroir
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }

      if (e.key === "Escape" && drawerOpen) {
        e.preventDefault();
        setDrawerOpen(false);
        return;
      }
      if (drawerOpen || !selectedClip) return;

      if (e.key === "ArrowLeft") {
        e.preventDefault();
        void handleReviewStatus(selectedClip, "rejected");
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        void handleReviewStatus(selectedClip, "approved");
      } else if (e.key.toLowerCase() === "e") {
        e.preventDefault();
        loadClipIntoDrawer(selectedClip);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        const idx = filteredClips.findIndex((c) => c.id === selectedClip.id);
        if (idx >= 0 && idx + 1 < filteredClips.length) {
          setSelectedClipId(filteredClips[idx + 1].id);
        }
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        const idx = filteredClips.findIndex((c) => c.id === selectedClip.id);
        if (idx > 0) {
          setSelectedClipId(filteredClips[idx - 1].id);
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drawerOpen, selectedClip, filteredClips]);

  // Lecture audio en boucle sur [editStart, editEnd]
  const audioAssetUrl = useMemo(() => {
    const p = timelineQuery.data?.vod.audio_path;
    if (!p) return null;
    try {
      return convertFileSrc(p);
    } catch {
      return null;
    }
  }, [timelineQuery.data?.vod.audio_path]);

  const toggleLoopPlay = () => {
    const el = mediaRef.current;
    if (!el) return;
    if (isLoopPlaying) {
      el.pause();
      setIsLoopPlaying(false);
    } else {
      if (el.currentTime < editStart || el.currentTime >= editEnd) {
        el.currentTime = editStart;
      }
      void el.play().then(() => setIsLoopPlaying(true)).catch(() => {
        toast.warning("Aperçu audio indisponible pour ce fichier.");
      });
    }
  };

  const handleTimeUpdate = () => {
    const el = mediaRef.current;
    if (!el) return;
    setPlayheadSec(el.currentTime);
    if (el.currentTime >= editEnd) {
      el.currentTime = editStart;
      if (isLoopPlaying) {
        void el.play();
      }
    }
  };

  const handleSaveEdits = async (andApprove = false) => {
    if (!selectedClip) return;
    setSaving(true);
    try {
      const hashtags = editTags
        .split(",")
        .map((t) => t.trim().replace(/^#/, ""))
        .filter(Boolean);

      const res = await commands.saveClipEdits({
        id: selectedClip.id,
        start_sec: editStart,
        end_sec: Math.max(editStart + 2, editEnd),
        hook: editHook,
        title: editTitle,
        caption: editCaption,
        hashtags,
        category: editCategory,
        layout_mode: editLayout,
        subtitle_style: editStyle,
        subtitles_override_json: JSON.stringify(editOverrides),
      });
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }

      // Sauvegarde aussi la zone facecam dans les réglages par défaut + profil streamer si présent
      if (renderSettingsQuery.data) {
        await commands.setRenderSettings({
          ...renderSettingsQuery.data,
          default_facecam: facecam,
        });
      }
      const profileId = timelineQuery.data?.project.streamer_profile_id;
      if (profileId) {
        await commands.saveProfileFacecam(profileId, facecam);
      }

      if (andApprove) {
        await commands.setClipReviewStatus(selectedClip.id, "approved");
      }

      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      toast.success(andApprove ? "Clip enregistré et validé !" : "Modifications enregistrées.");
      if (andApprove) {
        setDrawerOpen(false);
      }
    } finally {
      setSaving(false);
    }
  };

  const handleRegenerateTitle = async () => {
    if (!selectedClip) return;
    setRegenerating(true);
    try {
      const res = await commands.regenerateClipTitle(selectedClip.id);
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      setEditHook(res.data.hook);
      setEditTitle(res.data.title);
      setEditCaption(res.data.caption);
      setEditTags(res.data.hashtags.join(", "));
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      await queryClient.invalidateQueries({ queryKey: ["costs-dashboard"] });
      toast.success("Accroche, titre et hashtags régénérés sur ce seul clip.");
    } finally {
      setRegenerating(false);
    }
  };

  const handlePromoteLocal = async () => {
    if (!selectedProjectId) return;
    setPromoting(true);
    try {
      const res = await commands.promoteProjectCandidates(selectedProjectId);
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      toast.success(`${res.data.length} clips créés depuis les candidats locaux.`);
    } finally {
      setPromoting(false);
    }
  };

  const activePreset: SubtitleStylePreset | undefined = useMemo(() => {
    return presetsQuery.data?.find((p) => p.id === editStyle) ?? presetsQuery.data?.[0];
  }, [presetsQuery.data, editStyle]);

  const approvedCount = allClips.filter((c) => c.review_status === "approved").length;
  const maxVodDur = Math.max(timelineQuery.data?.vod.duration_sec ?? 600, editEnd + 10);

  if (!selectedProjectId) {
    return (
      <div className="space-y-6">
        <header className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Revue des clips</h1>
          <p className="text-sm text-fg-muted">
            Validez, rejetez ou éditez vos propositions 9:16 au clavier (← / → / E).
          </p>
        </header>
        <EmptyState
          icon={Film}
          title="Aucun projet actif"
          description="Créez et analysez d'abord une VOD ou la démo 10 min."
          actions={
            <Button variant="primary" onClick={() => setActiveScreen("home")}>
              Retour à l'Accueil
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Revue des clips</h1>
          <p className="text-sm text-fg-muted">
            Raccourcis clavier : <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">←</kbd> rejeter ·{" "}
            <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">→</kbd> valider ·{" "}
            <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">E</kbd> éditer
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {projectsQuery.data && projectsQuery.data.length > 1 && (
            <select
              value={selectedProjectId}
              onChange={(e) => {
                setActiveProjectId(e.target.value);
                setSelectedClipId(null);
                setDrawerOpen(false);
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

          <Button
            variant="primary"
            size="sm"
            disabled={approvedCount === 0}
            onClick={() => setActiveScreen("export")}
          >
            <Download className="h-3.5 w-3.5" />
            Exporter les validés ({approvedCount})
          </Button>
        </div>
      </header>

      {/* Filtres */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap gap-1.5">
          {(
            [
              ["all", `Tous (${allClips.length})`],
              ["pending", `À relire (${allClips.filter((c) => c.review_status === "pending").length})`],
              ["approved", `Validés (${approvedCount})`],
              ["rejected", `Rejetés (${allClips.filter((c) => c.review_status === "rejected").length})`],
            ] as const
          ).map(([id, label]) => (
            <Button
              key={id}
              size="sm"
              variant={filter === id ? "primary" : "secondary"}
              onClick={() => setFilter(id)}
            >
              {label}
            </Button>
          ))}
        </div>
      </div>

      {/* État vide ou promotion depuis candidats locaux */}
      {allClips.length === 0 ? (
        <EmptyState
          icon={Film}
          title="Aucun clip prêt pour la revue"
          description={
            (timelineQuery.data?.candidates.length ?? 0) > 0
              ? `${timelineQuery.data?.candidates.length} candidats locaux ont été détectés. Promouvez-les en clips pour les éditer et les exporter en 9:16 (0 token).`
              : "Lancez l'analyse du projet (locale ou complète) pour générer des propositions de clips."
          }
          actions={
            (timelineQuery.data?.candidates.length ?? 0) > 0 ? (
              <Button variant="primary" disabled={promoting} onClick={() => void handlePromoteLocal()}>
                <Sparkles className="h-4 w-4" />
                {promoting ? "Création des clips…" : "Créer des clips depuis les candidats locaux"}
              </Button>
            ) : (
              <Button variant="primary" onClick={() => setActiveScreen("analysis")}>
                Ouvrir l'Analyse
              </Button>
            )
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {filteredClips.map((clip) => {
            const isSelected = selectedClip?.id === clip.id;
            const dur = Math.max(1, Math.round(clip.end_sec - clip.start_sec));
            return (
              <motion.div
                key={clip.id}
                whileHover={{ y: -3 }}
                transition={{ duration: 0.14 }}
                onClick={() => setSelectedClipId(clip.id)}
                className={`group cursor-pointer rounded-2xl border p-3 transition-colors cf-glass ${
                  isSelected
                    ? "border-accent-cyan ring-1 ring-accent-cyan/40"
                    : "border-border/75 hover:border-border-strong"
                }`}
              >
                {/* Carte verticale 9:16 */}
                <div className="relative aspect-[9/16] w-full overflow-hidden rounded-xl border border-border/60 bg-gradient-to-b from-surface-base via-surface-elevated to-surface-base p-3 flex flex-col justify-between">
                  {/* Top badges */}
                  <div className="flex items-center justify-between gap-1.5">
                    <Badge variant="cyan" className="uppercase text-[10px]">
                      {clip.category}
                    </Badge>
                    <div className="flex items-center gap-1">
                      {clip.score != null && (
                        <Badge variant="violet" className="font-mono text-[10px]">
                          {clip.score}/100
                        </Badge>
                      )}
                      <Badge variant="default" className="font-mono text-[10px]">
                        {dur}s
                      </Badge>
                    </div>
                  </div>

                  {/* Aperçu central : schéma mise en page + hook */}
                  <div className="my-auto space-y-2.5 text-center">
                    {clip.hook && (
                      <div className="mx-auto max-w-[92%] rounded-lg bg-accent-violet/85 px-2.5 py-1.5 text-[11px] font-bold uppercase tracking-wide text-white shadow">
                        {clip.hook}
                      </div>
                    )}
                    <div className="mx-auto w-24 rounded-lg border border-dashed border-border-strong/70 bg-surface-subtle/40 px-2 py-3 text-[10px] uppercase tracking-wider text-fg-subtle">
                      {clip.layout_mode} · 9:16
                    </div>
                  </div>

                  {/* Pied de carte 9:16 : statut & horodatage */}
                  <div className="space-y-1.5">
                    <p className="line-clamp-2 text-xs font-semibold text-fg leading-snug">
                      {clip.title || "Sans titre"}
                    </p>
                    <div className="flex items-center justify-between text-[10px] text-fg-subtle font-mono">
                      <span>
                        {formatDuration(clip.start_sec)} – {formatDuration(clip.end_sec)}
                      </span>
                      <Badge
                        variant={
                          clip.review_status === "approved"
                            ? "success"
                            : clip.review_status === "rejected"
                              ? "warning"
                              : "default"
                        }
                        className="text-[10px]"
                      >
                        {clip.review_status === "approved"
                          ? "Validé"
                          : clip.review_status === "rejected"
                            ? "Rejeté"
                            : "À relire"}
                      </Badge>
                    </div>
                  </div>
                </div>

                {/* Actions rapides */}
                <div className="mt-2.5 flex items-center justify-between gap-1.5">
                  <Button
                    size="sm"
                    variant="ghost"
                    className="flex-1 text-status-danger hover:bg-status-danger/10"
                    onClick={(e) => {
                      e.stopPropagation();
                      void handleReviewStatus(clip, "rejected");
                    }}
                  >
                    <X className="h-3.5 w-3.5" />
                    Rejeter
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={(e) => {
                      e.stopPropagation();
                      setSelectedClipId(clip.id);
                      loadClipIntoDrawer(clip);
                    }}
                  >
                    <Edit3 className="h-3.5 w-3.5" />
                    Éditer
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="flex-1 text-status-success hover:bg-status-success/10"
                    onClick={(e) => {
                      e.stopPropagation();
                      void handleReviewStatus(clip, "approved");
                    }}
                  >
                    <Check className="h-3.5 w-3.5" />
                    Valider
                  </Button>
                </div>
              </motion.div>
            );
          })}
        </div>
      )}

      {/* Tiroir latéral d'édition */}
      <AnimatePresence>
        {drawerOpen && selectedClip && (
          <>
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onClick={() => setDrawerOpen(false)}
              className="fixed inset-0 z-40 bg-black/55 backdrop-blur-sm"
            />
            <motion.aside
              initial={{ x: "100%" }}
              animate={{ x: 0 }}
              exit={{ x: "100%" }}
              transition={{ type: "spring", stiffness: 360, damping: 34 }}
              className="fixed bottom-0 right-0 top-11 z-50 flex w-[min(780px,94vw)] flex-col border-l border-border bg-surface-base/95 shadow-2xl backdrop-blur-xl"
            >
              {/* En-tête tiroir */}
              <div className="flex items-center justify-between border-b border-border/70 px-5 py-3.5">
                <div className="min-w-0">
                  <h2 className="truncate text-base font-semibold text-fg">
                    Éditeur de clip · {formatDuration(editStart)} → {formatDuration(editEnd)} (
                    {Math.max(1, Math.round(editEnd - editStart))} s)
                  </h2>
                  <p className="text-xs text-fg-muted">
                    Découpe en boucle, mise en page 9:16, zone facecam, sous-titres mot à mot et accroche.
                  </p>
                </div>
                <Button size="icon" variant="ghost" onClick={() => setDrawerOpen(false)}>
                  <X className="h-4 w-4" />
                </Button>
              </div>

              {/* Corps scrollable */}
              <div className="grid flex-1 grid-cols-1 gap-5 overflow-y-auto p-5 md:grid-cols-5">
                {/* Colonne gauche : aperçu 9:16 + zones de sécurité TikTok + lecteur boucle */}
                <div className="space-y-3 md:col-span-2">
                  <div className="flex items-center justify-between text-xs">
                    <span className="font-semibold text-fg">Aperçu 9:16</span>
                    <button
                      type="button"
                      onClick={() => setShowSafeZones((v) => !v)}
                      className="inline-flex items-center gap-1 text-[11px] text-accent-cyan hover:underline"
                    >
                      <Shield className="h-3 w-3" />
                      {showSafeZones ? "Masquer zones TikTok" : "Afficher zones TikTok"}
                    </button>
                  </div>

                  <div className="relative mx-auto aspect-[9/16] w-full max-w-[260px] overflow-hidden rounded-2xl border border-border-strong bg-slate-950">
                    {/* Rendu schématique Split / Fit / Fill */}
                    {editLayout === "split" && (
                      <div className="flex h-full w-full flex-col">
                        <div className="flex h-1/3 items-center justify-center border-b border-white/10 bg-violet-950/60 text-[11px] text-violet-200">
                          Facecam ({Math.round(facecam.w * 100)}%×{Math.round(facecam.h * 100)}%)
                        </div>
                        <div className="flex h-2/3 items-center justify-center bg-slate-900/90 text-xs text-slate-300">
                          Jeu (recadré bas)
                        </div>
                      </div>
                    )}
                    {editLayout === "fit" && (
                      <div className="relative flex h-full w-full items-center justify-center bg-gradient-to-b from-violet-950/70 via-slate-900 to-cyan-950/70">
                        <div className="absolute left-3 top-5 flex h-14 w-14 items-center justify-center rounded-xl border border-accent-cyan/60 bg-violet-900/80 text-[9px] text-white">
                          Cam
                        </div>
                        <div className="flex aspect-video w-full items-center justify-center border-y border-white/15 bg-slate-950/90 text-xs text-slate-200">
                          Jeu 16:9 centré
                        </div>
                      </div>
                    )}
                    {editLayout === "fill" && (
                      <div className="flex h-full w-full items-center justify-center bg-slate-900 text-xs text-slate-200">
                        Fill 9:16 (pan {Math.round(editOverrides.pan_x * 100)}%)
                      </div>
                    )}

                    {/* Accroche (2-3 premières secondes) */}
                    {editHook.trim() && (
                      <div className="pointer-events-none absolute left-3 right-11 top-[14%] rounded-lg bg-slate-950/85 px-2.5 py-1.5 text-center text-[11px] font-bold uppercase text-white">
                        {editHook}
                      </div>
                    )}

                    {/* Aperçu sous-titre actif */}
                    <div className="pointer-events-none absolute bottom-[24%] left-3 right-11 text-center">
                      <span
                        className="inline-block rounded px-2 py-0.5 text-xs font-extrabold"
                        style={{
                          fontFamily: editOverrides.font_name || activePreset?.font_name || "sans-serif",
                          color: editOverrides.primary_color_hex || activePreset?.primary_color_hex || "#FFFFFF",
                          backgroundColor:
                            activePreset?.border_style === 3 ? "rgba(9,9,11,0.85)" : "transparent",
                          textShadow: "0 2px 4px rgba(0,0,0,0.9)",
                        }}
                      >
                        QUEL{" "}
                        <span
                          style={{
                            color:
                              editOverrides.highlight_color_hex ||
                              activePreset?.highlight_color_hex ||
                              "#FACC15",
                          }}
                        >
                          CLUTCH
                        </span>{" "}
                        INCROYABLE
                      </span>
                    </div>

                    {/* Zones de sécurité TikTok (bas 20 % + bande droite 15 %) */}
                    {showSafeZones && (
                      <>
                        <div className="pointer-events-none absolute bottom-0 left-0 right-0 flex h-[20%] items-center justify-center border-t border-dashed border-red-400/70 bg-red-500/15 text-[9px] text-red-200">
                          Zone UI TikTok (20 % bas)
                        </div>
                        <div className="pointer-events-none absolute bottom-[20%] right-0 top-[25%] flex w-[15%] items-center justify-center border-l border-dashed border-red-400/70 bg-red-500/15">
                          <span className="-rotate-90 whitespace-nowrap text-[8px] text-red-200">
                            Boutons TikTok
                          </span>
                        </div>
                      </>
                    )}
                  </div>

                  {/* Lecteur audio en boucle + poignées de découpe */}
                  <Card className="p-3 space-y-2.5">
                    <div className="flex items-center justify-between">
                      <Button size="sm" variant="secondary" onClick={toggleLoopPlay} disabled={!audioAssetUrl}>
                        {isLoopPlaying ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
                        {isLoopPlaying ? "Pause boucle" : "Lire en boucle"}
                      </Button>
                      <span className="font-mono text-[11px] text-fg-muted">
                        {formatDuration(playheadSec)}
                      </span>
                    </div>
                    {audioAssetUrl && (
                      <audio
                        ref={mediaRef}
                        src={audioAssetUrl}
                        onTimeUpdate={handleTimeUpdate}
                        onEnded={() => setIsLoopPlaying(false)}
                        className="hidden"
                      />
                    )}
                    <div className="space-y-1.5 text-xs">
                      <label className="flex justify-between text-fg-muted">
                        <span>Début</span>
                        <span className="font-mono text-fg">{editStart.toFixed(1)} s</span>
                      </label>
                      <input
                        type="range"
                        min={0}
                        max={Math.max(1, editEnd - 2)}
                        step={0.5}
                        value={editStart}
                        onChange={(e) => setEditStart(Number(e.target.value))}
                        className="w-full accent-violet-500"
                      />
                      <label className="flex justify-between text-fg-muted">
                        <span>Fin</span>
                        <span className="font-mono text-fg">{editEnd.toFixed(1)} s</span>
                      </label>
                      <input
                        type="range"
                        min={editStart + 2}
                        max={maxVodDur}
                        step={0.5}
                        value={editEnd}
                        onChange={(e) => setEditEnd(Number(e.target.value))}
                        className="w-full accent-cyan-400"
                      />
                    </div>
                  </Card>
                </div>

                {/* Colonne droite : métadonnées, mise en page, facecam, sous-titres */}
                <div className="space-y-4 md:col-span-3">
                  {/* Métadonnées + Régénération ciblée Gemini */}
                  <div className="space-y-2.5 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-semibold text-fg">Accroche, titre & légende</span>
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={regenerating}
                        onClick={() => void handleRegenerateTitle()}
                      >
                        {regenerating ? (
                          <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Wand2 className="h-3.5 w-3.5 text-accent-cyan" />
                        )}
                        Régénérer (1 clip)
                      </Button>
                    </div>
                    <input
                      value={editHook}
                      onChange={(e) => setEditHook(e.target.value)}
                      placeholder="Accroche affichée 2–3 s (8 mots max)"
                      className="h-9 w-full rounded-lg border border-border bg-surface-base/80 px-3 text-xs text-fg"
                    />
                    <input
                      value={editTitle}
                      onChange={(e) => setEditTitle(e.target.value)}
                      placeholder="Titre (60 caractères max)"
                      className="h-9 w-full rounded-lg border border-border bg-surface-base/80 px-3 text-xs text-fg"
                    />
                    <textarea
                      rows={2}
                      value={editCaption}
                      onChange={(e) => setEditCaption(e.target.value)}
                      placeholder="Description TikTok…"
                      className="w-full rounded-lg border border-border bg-surface-base/80 p-2.5 text-xs text-fg"
                    />
                    <input
                      value={editTags}
                      onChange={(e) => setEditTags(e.target.value)}
                      placeholder="Hashtags séparés par virgules (ex. twitch, clutch, gaming)"
                      className="h-9 w-full rounded-lg border border-border bg-surface-base/80 px-3 font-mono text-xs text-fg"
                    />
                  </div>

                  {/* Mise en page & zone facecam */}
                  <div className="space-y-3 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                    <div className="flex items-center gap-2 text-xs font-semibold text-fg">
                      <Layout className="h-3.5 w-3.5 text-accent-violet" />
                      Mise en page 9:16 & zone facecam
                    </div>
                    <div className="grid grid-cols-3 gap-2">
                      {(["split", "fit", "fill"] as const).map((m) => (
                        <Button
                          key={m}
                          size="sm"
                          variant={editLayout === m ? "primary" : "secondary"}
                          onClick={() => setEditLayout(m)}
                          className="capitalize"
                        >
                          {m}
                        </Button>
                      ))}
                    </div>

                    {editLayout === "fill" ? (
                      <div className="space-y-1 text-xs">
                        <label className="flex justify-between text-fg-muted">
                          <span>Panoramique horizontal</span>
                          <span className="font-mono">{Math.round(editOverrides.pan_x * 100)} %</span>
                        </label>
                        <input
                          type="range"
                          min={-1}
                          max={1}
                          step={0.05}
                          value={editOverrides.pan_x}
                          onChange={(e) =>
                            setEditOverrides({ ...editOverrides, pan_x: Number(e.target.value) })
                          }
                          className="w-full accent-cyan-400"
                        />
                      </div>
                    ) : (
                      <div className="grid grid-cols-2 gap-2 text-xs">
                        {(
                          [
                            ["x", "Facecam X"],
                            ["y", "Facecam Y"],
                            ["w", "Largeur"],
                            ["h", "Hauteur"],
                          ] as const
                        ).map(([k, lbl]) => (
                          <div key={k} className="space-y-0.5">
                            <label className="flex justify-between text-[11px] text-fg-muted">
                              <span>{lbl}</span>
                              <span className="font-mono">{Math.round(facecam[k] * 100)}%</span>
                            </label>
                            <input
                              type="range"
                              min={k === "w" || k === "h" ? 0.1 : 0}
                              max={k === "w" || k === "h" ? 0.8 : 0.9}
                              step={0.02}
                              value={facecam[k]}
                              onChange={(e) =>
                                setFacecam({ ...facecam, [k]: Number(e.target.value) })
                              }
                              className="w-full accent-violet-500"
                            />
                          </div>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Style des sous-titres (5 préréglages + couleurs/police) */}
                  <div className="space-y-3 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                    <div className="flex items-center gap-2 text-xs font-semibold text-fg">
                      <TypeIcon className="h-3.5 w-3.5 text-accent-cyan" />
                      Style des sous-titres mot à mot
                    </div>
                    <select
                      value={editStyle}
                      onChange={(e) => setEditStyle(e.target.value)}
                      className="h-9 w-full rounded-lg border border-border bg-surface-elevated px-3 text-xs text-fg"
                    >
                      {presetsQuery.data?.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                    <div className="grid grid-cols-3 gap-2 text-xs">
                      <div className="space-y-1">
                        <label className="text-[11px] text-fg-muted">Police</label>
                        <input
                          value={editOverrides.font_name ?? activePreset?.font_name ?? "Segoe UI"}
                          onChange={(e) =>
                            setEditOverrides({ ...editOverrides, font_name: e.target.value })
                          }
                          className="h-8 w-full rounded-lg border border-border bg-surface-base/80 px-2 text-xs text-fg"
                        />
                      </div>
                      <div className="space-y-1">
                        <label className="text-[11px] text-fg-muted">Texte</label>
                        <input
                          type="color"
                          value={
                            editOverrides.primary_color_hex ??
                            activePreset?.primary_color_hex ??
                            "#FFFFFF"
                          }
                          onChange={(e) =>
                            setEditOverrides({
                              ...editOverrides,
                              primary_color_hex: e.target.value,
                            })
                          }
                          className="h-8 w-full cursor-pointer rounded-lg border border-border bg-transparent"
                        />
                      </div>
                      <div className="space-y-1">
                        <label className="text-[11px] text-fg-muted">Mot actif</label>
                        <input
                          type="color"
                          value={
                            editOverrides.highlight_color_hex ??
                            activePreset?.highlight_color_hex ??
                            "#FACC15"
                          }
                          onChange={(e) =>
                            setEditOverrides({
                              ...editOverrides,
                              highlight_color_hex: e.target.value,
                            })
                          }
                          className="h-8 w-full cursor-pointer rounded-lg border border-border bg-transparent"
                        />
                      </div>
                    </div>

                    {/* Éditeur des lignes de sous-titres */}
                    <div className="space-y-1.5 border-t border-border/50 pt-2.5">
                      <div className="flex items-center justify-between">
                        <span className="text-[11px] font-medium text-fg-muted">
                          Lignes de sous-titres ({editOverrides.cues.length})
                        </span>
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() =>
                            setEditOverrides({
                              ...editOverrides,
                              cues: [
                                ...editOverrides.cues,
                                {
                                  start_sec: editStart,
                                  end_sec: Math.min(editEnd, editStart + 3),
                                  text: "Nouveau sous-titre",
                                },
                              ],
                            })
                          }
                        >
                          <Plus className="h-3 w-3" />
                          Ajouter
                        </Button>
                      </div>
                      <div className="max-h-36 space-y-1.5 overflow-y-auto pr-1">
                        {editOverrides.cues.map((cue, idx) => (
                          <div key={idx} className="flex items-center gap-1.5">
                            <input
                              type="number"
                              step={0.2}
                              value={cue.start_sec}
                              onChange={(e) => {
                                const next = [...editOverrides.cues];
                                next[idx] = { ...cue, start_sec: Number(e.target.value) };
                                setEditOverrides({ ...editOverrides, cues: next });
                              }}
                              className="h-7 w-16 rounded border border-border bg-surface-base/80 px-1.5 font-mono text-[11px] text-fg"
                            />
                            <input
                              type="text"
                              value={cue.text}
                              onChange={(e) => {
                                const next = [...editOverrides.cues];
                                next[idx] = { ...cue, text: e.target.value };
                                setEditOverrides({ ...editOverrides, cues: next });
                              }}
                              className="h-7 flex-1 rounded border border-border bg-surface-base/80 px-2 text-xs text-fg"
                            />
                            <button
                              type="button"
                              onClick={() => {
                                const next = editOverrides.cues.filter((_, i) => i !== idx);
                                setEditOverrides({ ...editOverrides, cues: next });
                              }}
                              className="p-1 text-fg-subtle hover:text-status-danger"
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </button>
                          </div>
                        ))}
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              {/* Pied de tiroir */}
              <div className="flex items-center justify-end gap-2 border-t border-border/70 px-5 py-3">
                <Button variant="secondary" disabled={saving} onClick={() => void handleSaveEdits(false)}>
                  <Save className="h-3.5 w-3.5" />
                  Enregistrer
                </Button>
                <Button variant="primary" disabled={saving} onClick={() => void handleSaveEdits(true)}>
                  <Check className="h-3.5 w-3.5" />
                  Enregistrer & valider
                </Button>
              </div>
            </motion.aside>
          </>
        )}
      </AnimatePresence>
    </div>
  );
}
