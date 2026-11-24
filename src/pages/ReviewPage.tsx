import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { toast } from "sonner";
import {
  Check,
  CheckSquare,
  Download,
  Edit3,
  ExternalLink,
  Film,
  KeyRound,
  Layout,
  Link2,
  Loader2,
  LogOut,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Save,
  Share2,
  Shield,
  Hash,
  Sparkles,
  Star,
  Square,
  Trash2,
  Type as TypeIcon,
  Upload,
  Video,
  Volume2,
  VolumeX,
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
type LayoutMode = "split" | "fit" | "fill";
type TikTokTab = "integrated" | "api";
type TikTokUploadMode = "inbox" | "direct";
type TikTokPrivacy =
  | "SELF_ONLY"
  | "MUTUAL_FOLLOW_FRIENDS"
  | "FOLLOWER_OF_CREATOR"
  | "PUBLIC_TO_EVERYONE";

interface EditableSubtitleCue {
  start_sec: number;
  end_sec: number;
  text: string;
}

interface ClipOverrides {
  cues: EditableSubtitleCue[];
  pan_x: number;
  font_name: string | null;
  primary_color_hex: string | null;
  highlight_color_hex: string | null;
  font_size: number | null;
  hide_subtitles: boolean;
  hide_hook: boolean;
}

interface ClipPreviewResult {
  video_path: string;
  preview_url: string;
  offset_sec: number;
  is_rendered: boolean;
}

interface TikTokPublishPrep {
  mp4_path: string;
  caption_text: string;
  studio_url: string;
}

interface TikTokStatus {
  client_configured: boolean;
  connected: boolean;
  client_key_preview: string | null;
  redirect_uri: string;
  use_pkce: boolean;
  creator_nickname: string | null;
  open_id: string | null;
  expires_at_iso: string | null;
}

interface TikTokAiSuggestion {
  descriptions: string[];
  hashtags: string[];
}

interface TikTokAutoInjectResult {
  mp4_path: string;
  file_name: string;
  file_size_bytes: number;
  auto_publish: boolean;
  message: string;
}

interface TikTokUploadResult {
  publish_id: string;
  mode: string;
  status: string;
  message: string;
}

const FAV_TAGS_STORAGE_KEY = "clipforge.tiktok.fav_tags";

function normalizeTag(raw: string): string {
  return raw.trim().replace(/^#+/, "").replace(/\s+/g, "");
}

function loadFavTags(): string[] {
  try {
    const raw = window.localStorage.getItem(FAV_TAGS_STORAGE_KEY);
    if (!raw) return ["twitch", "clip", "gaming", "fyp"];
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      return parsed.map((v) => normalizeTag(String(v))).filter(Boolean);
    }
  } catch {
    // ignore
  }
  return ["twitch", "clip", "gaming", "fyp"];
}

const DEFAULT_FACECAM: FacecamRect = { x: 0.02, y: 0.62, w: 0.24, h: 0.34 };

function extractErrMsg(err: unknown): string {
  if (typeof err === "object" && err && "message" in err) {
    return String((err as { message: unknown }).message);
  }
  return String(err);
}

function parseClipOverrides(clip: ClipRecord): ClipOverrides {
  const base: ClipOverrides = {
    cues: [],
    pan_x: 0,
    font_name: null,
    primary_color_hex: null,
    highlight_color_hex: null,
    font_size: null,
    hide_subtitles: false,
    hide_hook: false,
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
        hide_subtitles: Boolean(parsed.hide_subtitles),
        hide_hook: Boolean(parsed.hide_hook),
      };
    } catch {
      // ignore invalid JSON
    }
  }
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

function buildDefaultTikTokCaption(clip: ClipRecord): string {
  const tags = clip.hashtags
    .map((t) => `#${t.trim().replace(/^#/, "")}`)
    .filter((t) => t.length > 1)
    .join(" ");
  const parts = [clip.title.trim(), clip.caption.trim(), tags].filter(Boolean);
  return parts.join("\n").slice(0, 2150);
}

export function ReviewPage() {
  const queryClient = useQueryClient();
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);

  const [filter, setFilter] = useState<ReviewFilter>("all");
  const [selectedClipId, setSelectedClipId] = useState<string | null>(null);
  const [bulkSelectedIds, setBulkSelectedIds] = useState<string[]>([]);
  const [bulkStyle, setBulkStyle] = useState<string>("bold_pop");
  const [bulkApplying, setBulkApplying] = useState(false);

  // Prévisualisation vidéo locale (HTTP Range 206 — 100 % compatible WebView2)
  const [previewCache, setPreviewCache] = useState<Record<string, ClipPreviewResult>>({});
  const [cardLoadingId, setCardLoadingId] = useState<string | null>(null);
  const [playingCardId, setPlayingCardId] = useState<string | null>(null);
  const [cardMuted, setCardMuted] = useState(false);
  const cardVideoRefs = useRef<Record<string, HTMLVideoElement | null>>({});

  // Tiroir d'édition
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [showSafeZones, setShowSafeZones] = useState(true);
  const [promoting, setPromoting] = useState(false);
  const [editStart, setEditStart] = useState(0);
  const [editEnd, setEditEnd] = useState(30);
  const [trimAnchorStart, setTrimAnchorStart] = useState(0);
  const [trimAnchorEnd, setTrimAnchorEnd] = useState(30);
  const [editHook, setEditHook] = useState("");
  const [editTitle, setEditTitle] = useState("");
  const [editCaption, setEditCaption] = useState("");
  const [editTags, setEditTags] = useState("");
  const [editCategory, setEditCategory] = useState("fun");
  const [editLayout, setEditLayout] = useState("fill");
  const [editStyle, setEditStyle] = useState("bold_pop");
  const [editOverrides, setEditOverrides] = useState<ClipOverrides>({
    cues: [],
    pan_x: 0,
    font_name: null,
    primary_color_hex: null,
    highlight_color_hex: null,
    font_size: null,
    hide_subtitles: false,
    hide_hook: false,
  });
  const [facecam, setFacecam] = useState<FacecamRect>(DEFAULT_FACECAM);
  const [saving, setSaving] = useState(false);
  const [regenerating, setRegenerating] = useState(false);

  // Lecteur du tiroir d'édition
  const mediaRef = useRef<HTMLAudioElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const [isLoopPlaying, setIsLoopPlaying] = useState(false);
  const [playheadSec, setPlayheadSec] = useState(0);
  const [videoPreview, setVideoPreview] = useState<ClipPreviewResult | null>(null);
  const [loadingPreview, setLoadingPreview] = useState(false);

  // Modal TikTok centrée (Onglet intégré sans clé API + Onglet OAuth2 avancé)
  const [tiktokModalOpen, setTiktokModalOpen] = useState(false);
  const [tiktokTab, setTiktokTab] = useState<TikTokTab>("integrated");
  const [tiktokClip, setTiktokClip] = useState<ClipRecord | null>(null);
  const [ttClientKey, setTtClientKey] = useState("");
  const [ttClientSecret, setTtClientSecret] = useState("");
  const [ttRedirectUri, setTtRedirectUri] = useState("http://127.0.0.1:48321/tiktok/callback");
  const [ttUsePkce, setTtUsePkce] = useState(true);
  const [ttShowConfig, setTtShowConfig] = useState(false);
  const [ttSavingCreds, setTtSavingCreds] = useState(false);
  const [ttConnecting, setTtConnecting] = useState(false);
  const [ttOpeningStudio, setTtOpeningStudio] = useState(false);
  const [ttUploadMode, setTtUploadMode] = useState<TikTokUploadMode>("inbox");
  const [ttPrivacy, setTtPrivacy] = useState<TikTokPrivacy>("SELF_ONLY");
  const [ttCaption, setTtCaption] = useState("");
  const [ttDisableComment, setTtDisableComment] = useState(false);
  const [ttDisableDuet, setTtDisableDuet] = useState(false);
  const [ttDisableStitch, setTtDisableStitch] = useState(false);
  const [ttUploading, setTtUploading] = useState(false);
  const [ttLastResult, setTtLastResult] = useState<TikTokUploadResult | null>(null);
  const [ttDescription, setTtDescription] = useState("");
  const [ttAiVariants, setTtAiVariants] = useState<string[]>([]);
  const [ttActiveTags, setTtActiveTags] = useState<string[]>([]);
  const [ttSuggestedTags, setTtSuggestedTags] = useState<string[]>([]);
  const [ttFavTags, setTtFavTags] = useState<string[]>(() => loadFavTags());
  const [ttCustomTag, setTtCustomTag] = useState("");
  const [ttAutoPublish, setTtAutoPublish] = useState(false);
  const [ttLoadingAi, setTtLoadingAi] = useState(false);
  const [ttInjectResult, setTtInjectResult] = useState<TikTokAutoInjectResult | null>(null);

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

  const tiktokStatusQuery = useQuery({
    queryKey: ["tiktok-status"],
    queryFn: async () => invoke<TikTokStatus>("tiktok_get_status"),
    retry: false,
  });

  useEffect(() => {
    if (tiktokStatusQuery.data?.redirect_uri) {
      setTtRedirectUri(tiktokStatusQuery.data.redirect_uri);
      setTtUsePkce(tiktokStatusQuery.data.use_pkce);
    }
  }, [tiktokStatusQuery.data]);

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

  useEffect(() => {
    const validIds = new Set(allClips.map((c) => c.id));
    setBulkSelectedIds((prev) => {
      const next = prev.filter((id) => validIds.has(id));
      return next.length === prev.length ? prev : next;
    });
  }, [allClips]);

  const pauseAllCardVideosExcept = (keepId: string | null) => {
    for (const [id, el] of Object.entries(cardVideoRefs.current)) {
      if (id !== keepId && el && !el.paused) {
        el.pause();
      }
    }
  };

  // Prévisualisation vidéo directe sur la carte (avant validation)
  const handleToggleCardPreview = async (clip: ClipRecord) => {
    setSelectedClipId(clip.id);

    if (playingCardId === clip.id) {
      const vid = cardVideoRefs.current[clip.id];
      if (vid) vid.pause();
      setPlayingCardId(null);
      return;
    }

    pauseAllCardVideosExcept(clip.id);

    const existing = previewCache[clip.id];
    if (existing?.preview_url) {
      setPlayingCardId(clip.id);
      const vid = cardVideoRefs.current[clip.id];
      if (vid) {
        void vid.play().catch((err) => {
          toast.error(`Lecture vidéo impossible : ${extractErrMsg(err)}`);
          setPlayingCardId(null);
        });
      }
      return;
    }

    setCardLoadingId(clip.id);
    try {
      const res = await invoke<ClipPreviewResult>("prepare_clip_preview", { clipId: clip.id });
      setPreviewCache((prev) => ({ ...prev, [clip.id]: res }));
      setPlayingCardId(clip.id);
    } catch (err) {
      toast.error(`Prévisualisation impossible : ${extractErrMsg(err)}`);
    } finally {
      setCardLoadingId((cur) => (cur === clip.id ? null : cur));
    }
  };

  const bulkSelectedSet = useMemo(() => new Set(bulkSelectedIds), [bulkSelectedIds]);
  const allFilteredSelected =
    filteredClips.length > 0 && filteredClips.every((c) => bulkSelectedSet.has(c.id));

  const toggleBulkSelect = (clipId: string) => {
    setBulkSelectedIds((prev) =>
      prev.includes(clipId) ? prev.filter((id) => id !== clipId) : [...prev, clipId],
    );
  };

  const toggleSelectAllFiltered = () => {
    if (allFilteredSelected) {
      const visibleIds = new Set(filteredClips.map((c) => c.id));
      setBulkSelectedIds((prev) => prev.filter((id) => !visibleIds.has(id)));
    } else {
      const merged = new Set(bulkSelectedIds);
      for (const c of filteredClips) merged.add(c.id);
      setBulkSelectedIds(Array.from(merged));
    }
  };

  const handleBulkLayout = async (layoutMode: LayoutMode) => {
    const targets = allClips.filter((c) => bulkSelectedSet.has(c.id));
    if (targets.length === 0) return;
    setBulkApplying(true);
    try {
      let updatedCount = 0;
      for (const clip of targets) {
        const res = await commands.saveClipEdits({
          id: clip.id,
          start_sec: clip.start_sec,
          end_sec: Math.max(clip.start_sec + 2, clip.end_sec),
          hook: clip.hook,
          title: clip.title,
          caption: clip.caption,
          hashtags: clip.hashtags,
          category: clip.category,
          layout_mode: layoutMode,
          subtitle_style: clip.subtitle_style,
          subtitles_override_json: clip.subtitles_override_json,
        });
        if (res.status === "error") {
          toast.error(`Erreur sur « ${clip.title} » : ${res.error.message}`);
          break;
        }
        updatedCount += 1;
      }
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      toast.success(
        `Mise en page ${layoutMode.toUpperCase()} appliquée à ${updatedCount} clip(s).`,
      );
    } finally {
      setBulkApplying(false);
    }
  };

  const handleBulkSubtitleStyle = async (styleId: string) => {
    const targets = allClips.filter((c) => bulkSelectedSet.has(c.id));
    if (targets.length === 0) return;
    setBulkApplying(true);
    try {
      let updatedCount = 0;
      for (const clip of targets) {
        const res = await commands.saveClipEdits({
          id: clip.id,
          start_sec: clip.start_sec,
          end_sec: Math.max(clip.start_sec + 2, clip.end_sec),
          hook: clip.hook,
          title: clip.title,
          caption: clip.caption,
          hashtags: clip.hashtags,
          category: clip.category,
          layout_mode: clip.layout_mode,
          subtitle_style: styleId,
          subtitles_override_json: clip.subtitles_override_json,
        });
        if (res.status === "error") {
          toast.error(`Erreur sur « ${clip.title} » : ${res.error.message}`);
          break;
        }
        updatedCount += 1;
      }
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      toast.success(`Style de sous-titres appliqué à ${updatedCount} clip(s).`);
    } finally {
      setBulkApplying(false);
    }
  };

  const handleBulkOverlays = async (
    patch: { hide_subtitles?: boolean; hide_hook?: boolean },
    label: string,
  ) => {
    const targets = allClips.filter((c) => bulkSelectedSet.has(c.id));
    if (targets.length === 0) return;
    setBulkApplying(true);
    try {
      let updatedCount = 0;
      for (const clip of targets) {
        const cur = parseClipOverrides(clip);
        const next: ClipOverrides = {
          ...cur,
          hide_subtitles: patch.hide_subtitles ?? cur.hide_subtitles,
          hide_hook: patch.hide_hook ?? cur.hide_hook,
        };
        const res = await commands.saveClipEdits({
          id: clip.id,
          start_sec: clip.start_sec,
          end_sec: Math.max(clip.start_sec + 2, clip.end_sec),
          hook: clip.hook,
          title: clip.title,
          caption: clip.caption,
          hashtags: clip.hashtags,
          category: clip.category,
          layout_mode: clip.layout_mode,
          subtitle_style: clip.subtitle_style,
          subtitles_override_json: JSON.stringify(next),
        });
        if (res.status === "error") {
          toast.error(`Erreur sur « ${clip.title} » : ${res.error.message}`);
          break;
        }
        updatedCount += 1;
      }
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      toast.success(`${label} appliqué à ${updatedCount} clip(s).`);
    } finally {
      setBulkApplying(false);
    }
  };

  const handleBulkReviewStatus = async (status: "approved" | "rejected") => {
    const targets = allClips.filter((c) => bulkSelectedSet.has(c.id));
    if (targets.length === 0) return;
    setBulkApplying(true);
    try {
      let updatedCount = 0;
      for (const clip of targets) {
        const res = await commands.setClipReviewStatus(clip.id, status);
        if (res.status === "error") {
          toast.error(res.error.message);
          break;
        }
        updatedCount += 1;
      }
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      toast.success(
        status === "approved"
          ? `${updatedCount} clip(s) validé(s) pour l'export.`
          : `${updatedCount} clip(s) rejeté(s).`,
      );
    } finally {
      setBulkApplying(false);
    }
  };

  const loadClipIntoDrawer = (clip: ClipRecord) => {
    pauseAllCardVideosExcept(null);
    setPlayingCardId(null);
    const s0 = Number(clip.start_sec.toFixed(1));
    const e0 = Number(clip.end_sec.toFixed(1));
    setEditStart(s0);
    setEditEnd(e0);
    setTrimAnchorStart(s0);
    setTrimAnchorEnd(e0);
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
    setVideoPreview(previewCache[clip.id] ?? null);
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

    const idx = filteredClips.findIndex((c) => c.id === clip.id);
    if (idx >= 0 && idx + 1 < filteredClips.length) {
      setSelectedClipId(filteredClips[idx + 1].id);
    }
  };

  // Raccourcis clavier
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

      if (e.key === "Escape") {
        if (tiktokModalOpen) {
          e.preventDefault();
          setTiktokModalOpen(false);
        } else if (drawerOpen) {
          e.preventDefault();
          setDrawerOpen(false);
        } else if (bulkSelectedIds.length > 0) {
          e.preventDefault();
          setBulkSelectedIds([]);
        }
        return;
      }
      if (drawerOpen || tiktokModalOpen) return;

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a" && filteredClips.length > 0) {
        e.preventDefault();
        setBulkSelectedIds(filteredClips.map((c) => c.id));
        return;
      }

      if (!selectedClip) return;

      if (e.code === "Space") {
        e.preventDefault();
        void handleToggleCardPreview(selectedClip);
      } else if (e.key === "ArrowLeft") {
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
  }, [drawerOpen, tiktokModalOpen, selectedClip, filteredClips, bulkSelectedIds.length, playingCardId, previewCache]);

  const audioAssetUrl = useMemo(() => {
    const p = timelineQuery.data?.vod.audio_path;
    if (!p) return null;
    try {
      return convertFileSrc(p);
    } catch {
      return null;
    }
  }, [timelineQuery.data?.vod.audio_path]);

  const videoAssetUrl = videoPreview?.preview_url ?? null;

  const handleLoadDrawerVideoPreview = async (clip: ClipRecord) => {
    setLoadingPreview(true);
    try {
      // Si l'utilisateur a ajusté Début/Fin dans le tiroir, on sauvegarde d'abord pour apercevoir la nouvelle coupe
      const startChanged = Math.abs(editStart - clip.start_sec) > 0.05;
      const endChanged = Math.abs(editEnd - clip.end_sec) > 0.05;
      if (startChanged || endChanged) {
        const hashtags = editTags
          .split(",")
          .map((t) => t.trim().replace(/^#/, ""))
          .filter(Boolean);
        const saveRes = await commands.saveClipEdits({
          id: clip.id,
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
        if (saveRes.status === "error") {
          toast.error(saveRes.error.message);
          return;
        }
        await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      }
      const res = await invoke<ClipPreviewResult>("prepare_clip_preview", { clipId: clip.id });
      setVideoPreview(res);
      setPreviewCache((prev) => ({ ...prev, [clip.id]: res }));
      setIsLoopPlaying(true);
      toast.success(
        res.is_rendered
          ? "Aperçu du MP4 exporté chargé."
          : "Aperçu vidéo chargé sur les bornes actuelles.",
      );
    } catch (err) {
      toast.error(`Aperçu vidéo impossible : ${extractErrMsg(err)}`);
    } finally {
      setLoadingPreview(false);
    }
  };

  const toggleLoopPlay = () => {
    const vid = videoRef.current;
    if (vid && videoPreview && videoAssetUrl) {
      if (isLoopPlaying) {
        vid.pause();
        setIsLoopPlaying(false);
      } else {
        void vid.play().then(() => setIsLoopPlaying(true)).catch((err) => {
          toast.error(`Lecture vidéo impossible : ${extractErrMsg(err)}`);
        });
      }
      return;
    }
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

  const handleDrawerVideoTimeUpdate = () => {
    const vid = videoRef.current;
    if (!vid || !videoPreview) return;
    setPlayheadSec(vid.currentTime + videoPreview.offset_sec);
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

  // --- TikTok Modal Actions (Assistant IA + Auto-injection intégrée) ---
  const ttFinalCaption = useMemo(() => {
    const desc = ttDescription.trim();
    const tags = ttActiveTags
      .map((t) => `#${normalizeTag(t)}`)
      .filter((t) => t.length > 1)
      .join(" ");
    return [desc, tags].filter(Boolean).join("\n").slice(0, 2150);
  }, [ttDescription, ttActiveTags]);

  const openTikTokModalForClip = (clip: ClipRecord | null) => {
    pauseAllCardVideosExcept(null);
    setPlayingCardId(null);
    setTiktokClip(clip);
    const clipDesc = clip
      ? [clip.caption.trim(), clip.title.trim()].find((v) => v.length > 0) ?? ""
      : "";
    setTtDescription(clipDesc);
    setTtCaption(clip ? buildDefaultTikTokCaption(clip) : "");
    setTtAiVariants([]);
    setTtCustomTag("");
    const clipTags = (clip?.hashtags ?? []).map(normalizeTag).filter(Boolean);
    const favs = loadFavTags();
    setTtFavTags(favs);
    const mergedPool = Array.from(new Set([...clipTags, ...favs]));
    setTtSuggestedTags(mergedPool);
    setTtActiveTags(clipTags.length > 0 ? clipTags : favs.slice(0, 4));
    setTtLastResult(null);
    setTtInjectResult(null);
    setTiktokTab("integrated");
    setTtShowConfig(!tiktokStatusQuery.data?.connected);
    setTiktokModalOpen(true);
  };

  const toggleTikTokTag = (tag: string) => {
    const norm = normalizeTag(tag);
    if (!norm) return;
    setTtActiveTags((prev) =>
      prev.includes(norm) ? prev.filter((t) => t !== norm) : [...prev, norm],
    );
  };

  const toggleFavTikTokTag = (tag: string) => {
    const norm = normalizeTag(tag);
    if (!norm) return;
    setTtFavTags((prev) => {
      const next = prev.includes(norm) ? prev.filter((t) => t !== norm) : [...prev, norm];
      try {
        window.localStorage.setItem(FAV_TAGS_STORAGE_KEY, JSON.stringify(next));
      } catch {
        // ignore
      }
      return next;
    });
  };

  const handleAddCustomTikTokTag = () => {
    const parts = ttCustomTag
      .split(/[\s,]+/)
      .map(normalizeTag)
      .filter(Boolean);
    if (parts.length === 0) return;
    setTtSuggestedTags((prev) => Array.from(new Set([...prev, ...parts])));
    setTtActiveTags((prev) => Array.from(new Set([...prev, ...parts])));
    setTtCustomTag("");
  };

  const handleSuggestTikTokAi = async (clip: ClipRecord) => {
    setTtLoadingAi(true);
    try {
      const suggestion = await invoke<TikTokAiSuggestion>("tiktok_suggest_copy", {
        clipId: clip.id,
      });
      setTtAiVariants(suggestion.descriptions);
      if (suggestion.descriptions.length > 0 && !ttDescription.trim()) {
        setTtDescription(suggestion.descriptions[0]);
      }
      const cleanTags = suggestion.hashtags.map(normalizeTag).filter(Boolean);
      if (cleanTags.length > 0) {
        setTtSuggestedTags((prev) => Array.from(new Set([...cleanTags, ...prev])));
        setTtActiveTags((prev) => Array.from(new Set([...prev, ...cleanTags.slice(0, 5)])));
      }
      await queryClient.invalidateQueries({ queryKey: ["costs-dashboard"] });
      toast.success(
        `${suggestion.descriptions.length} variantes de description + ${cleanTags.length} hashtags proposés par Gemini.`,
      );
    } catch (err) {
      toast.error(`Suggestions IA TikTok : ${extractErrMsg(err)}`);
    } finally {
      setTtLoadingAi(false);
    }
  };

  const handleOpenIntegratedTikTokWindow = async () => {
    setTtOpeningStudio(true);
    try {
      await invoke("tiktok_open_studio_window");
      toast.success("Fenêtre TikTok intégrée ouverte ! Connecte-toi une seule fois : ta session reste enregistrée dans ClipForge.");
    } catch (err) {
      toast.error(`Fenêtre TikTok : ${extractErrMsg(err)}`);
    } finally {
      setTtOpeningStudio(false);
    }
  };

  const handleIntegratedPublishTikTok = async (clip: ClipRecord) => {
    setTtUploading(true);
    setTtInjectResult(null);
    try {
      const finalText = ttFinalCaption.trim() || ttCaption.trim();
      if (finalText && navigator.clipboard) {
        await navigator.clipboard.writeText(finalText).catch(() => {});
      }
      if (!clip.output_mp4_path) {
        toast.info("Encodage du MP4 9:16 puis auto-injection dans TikTok Studio…");
      } else {
        toast.info("Auto-injection du MP4 + description + # dans TikTok Studio…");
      }
      const res = await invoke<TikTokAutoInjectResult>("tiktok_auto_inject_upload", {
        clipId: clip.id,
        captionText: finalText,
        autoPublish: ttAutoPublish,
      });
      setTtInjectResult(res);
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      toast.success(res.message);
    } catch (err) {
      toast.error(`Auto-injection TikTok : ${extractErrMsg(err)}`);
    } finally {
      setTtUploading(false);
    }
  };

  const handleOpenExternalBrowserTikTok = async (clip: ClipRecord) => {
    try {
      if (!clip.output_mp4_path) {
        const qRes = await commands.enqueueClipRender(clip.id);
        if (qRes.status === "error") {
          toast.error(qRes.error.message);
          return;
        }
        toast.info("Export MP4 lancé dans File d'export ! Revenez cliquer dès qu'il est terminé.");
        setTiktokModalOpen(false);
        setActiveScreen("export");
        return;
      }
      const prep = await invoke<TikTokPublishPrep>("open_tiktok_publish", { clipId: clip.id });
      const toCopy = ttFinalCaption.trim() || ttCaption.trim() || prep.caption_text;
      if (toCopy && navigator.clipboard) {
        await navigator.clipboard.writeText(toCopy).catch(() => {});
      }
      toast.success("TikTok Studio ouvert dans votre navigateur + MP4 sélectionné + légende copiée !");
    } catch (err) {
      toast.error(`TikTok Studio : ${extractErrMsg(err)}`);
    }
  };

  const handleSaveTikTokCreds = async () => {
    setTtSavingCreds(true);
    try {
      await invoke<TikTokStatus>("tiktok_save_credentials", {
        input: {
          client_key: ttClientKey,
          client_secret: ttClientSecret,
          redirect_uri: ttRedirectUri,
          use_pkce: ttUsePkce,
        },
      });
      await queryClient.invalidateQueries({ queryKey: ["tiktok-status"] });
      setTtClientSecret("");
      toast.success("Identifiants TikTok enregistrés dans le Gestionnaire d'identifiants Windows.");
    } catch (err) {
      toast.error(`Enregistrement TikTok : ${extractErrMsg(err)}`);
    } finally {
      setTtSavingCreds(false);
    }
  };

  const handleConnectTikTokOAuth = async () => {
    setTtConnecting(true);
    try {
      toast.info("Ouverture de TikTok dans votre navigateur… Validez l'autorisation pour revenir automatiquement.");
      const status = await invoke<TikTokStatus>("tiktok_connect_oauth");
      await queryClient.invalidateQueries({ queryKey: ["tiktok-status"] });
      setTtShowConfig(false);
      toast.success(
        status.creator_nickname
          ? `Compte TikTok connecté : ${status.creator_nickname} !`
          : "Compte TikTok connecté !",
      );
    } catch (err) {
      toast.error(`Connexion TikTok : ${extractErrMsg(err)}`);
    } finally {
      setTtConnecting(false);
    }
  };

  const handleDisconnectTikTok = async () => {
    try {
      await invoke<TikTokStatus>("tiktok_disconnect");
      await queryClient.invalidateQueries({ queryKey: ["tiktok-status"] });
      toast.info("Compte TikTok API déconnecté.");
    } catch (err) {
      toast.error(`Déconnexion TikTok : ${extractErrMsg(err)}`);
    }
  };

  const handleDirectUploadTikTok = async () => {
    if (!tiktokClip) return;
    setTtUploading(true);
    setTtLastResult(null);
    try {
      toast.info(
        tiktokClip.output_mp4_path
          ? "Envoi du MP4 vers TikTok en cours…"
          : "Encodage MP4 9:16 + upload direct vers TikTok en cours…",
      );
      const res = await invoke<TikTokUploadResult>("tiktok_upload_clip", {
        input: {
          clip_id: tiktokClip.id,
          mode: ttUploadMode,
          title: ttFinalCaption.trim() || ttCaption,
          privacy_level: ttPrivacy,
          disable_comment: ttDisableComment,
          disable_duet: ttDisableDuet,
          disable_stitch: ttDisableStitch,
        },
      });
      setTtLastResult(res);
      await queryClient.invalidateQueries({ queryKey: ["project-timeline", selectedProjectId] });
      toast.success(res.message);
    } catch (err) {
      toast.error(`Upload TikTok échoué : ${extractErrMsg(err)}`);
    } finally {
      setTtUploading(false);
    }
  };

  const activePreset: SubtitleStylePreset | undefined = useMemo(() => {
    return presetsQuery.data?.find((p) => p.id === editStyle) ?? presetsQuery.data?.[0];
  }, [presetsQuery.data, editStyle]);

  const approvedCount = allClips.filter((c) => c.review_status === "approved").length;
  const maxVodDur = Math.max(timelineQuery.data?.vod.duration_sec ?? 600, editEnd + 10);
  const ttStatus = tiktokStatusQuery.data;

  if (!selectedProjectId) {
    return (
      <div className="space-y-6">
        <header className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Revue des clips</h1>
          <p className="text-sm text-fg-muted">
            Prévisualisez, validez, rejetez ou publiez vos clips 9:16 sur TikTok.
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
    <div className="space-y-5">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Revue des clips</h1>
          <p className="text-sm text-fg-muted">
            Raccourcis : <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">Espace</kbd> lire ·{" "}
            <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">←</kbd> rejeter ·{" "}
            <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">→</kbd> valider ·{" "}
            <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">E</kbd> éditer ·{" "}
            <kbd className="rounded bg-surface-subtle px-1.5 py-0.5 font-mono text-[11px]">Ctrl+A</kbd> tout cocher
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {projectsQuery.data && projectsQuery.data.length > 1 && (
            <select
              value={selectedProjectId}
              onChange={(e) => {
                pauseAllCardVideosExcept(null);
                setPlayingCardId(null);
                setActiveProjectId(e.target.value);
                setSelectedClipId(null);
                setBulkSelectedIds([]);
                setDrawerOpen(false);
              }}
              className="h-9 max-w-[280px] truncate rounded-xl border border-border bg-surface-elevated px-3 text-xs text-fg"
            >
              {projectsQuery.data.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          )}

          <Button
            variant="outline"
            size="sm"
            onClick={() => openTikTokModalForClip(selectedClip)}
          >
            <Link2 className="h-3.5 w-3.5 text-accent-cyan" />
            {ttStatus?.connected
              ? `TikTok (${ttStatus.creator_nickname ?? "Connecté"})`
              : "Lier TikTok"}
          </Button>

          <Button
            variant="primary"
            size="sm"
            disabled={approvedCount === 0}
            onClick={() => setActiveScreen("export")}
          >
            <Download className="h-3.5 w-3.5" />
            Exporter ({approvedCount})
          </Button>
        </div>
      </header>

      {/* Filtres + bouton multi-sélection */}
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

        {filteredClips.length > 0 && (
          <Button size="sm" variant="outline" onClick={toggleSelectAllFiltered}>
            {allFilteredSelected ? (
              <CheckSquare className="h-3.5 w-3.5 text-accent-cyan" />
            ) : (
              <Square className="h-3.5 w-3.5" />
            )}
            {allFilteredSelected
              ? "Tout désélectionner"
              : `Tout sélectionner (${filteredClips.length})`}
          </Button>
        )}
      </div>

      {/* Barre d'édition en lot */}
      <AnimatePresence>
        {bulkSelectedIds.length > 0 && (
          <motion.div
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.14 }}
            className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-accent-cyan/50 bg-accent-violet/10 px-4 py-3 cf-glass"
          >
            <div className="flex items-center gap-2 text-xs">
              <Badge variant="cyan" className="font-mono">
                {bulkSelectedIds.length} sélectionné(s)
              </Badge>
              <button
                type="button"
                onClick={() => setBulkSelectedIds([])}
                className="text-fg-muted hover:text-fg underline-offset-2 hover:underline"
              >
                Effacer
              </button>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <div className="flex items-center gap-1 rounded-xl border border-border/70 bg-surface-base/60 p-1">
                <span className="px-2 text-[11px] text-fg-muted">Format :</span>
                {(["split", "fit", "fill"] as const).map((mode) => (
                  <Button
                    key={mode}
                    size="sm"
                    variant="secondary"
                    disabled={bulkApplying}
                    onClick={() => void handleBulkLayout(mode)}
                    className="h-7 px-2.5 text-[11px] uppercase"
                  >
                    {mode}
                  </Button>
                ))}
              </div>

              {presetsQuery.data && presetsQuery.data.length > 0 && (
                <div className="flex items-center gap-1 rounded-xl border border-border/70 bg-surface-base/60 p-1">
                  <select
                    value={bulkStyle}
                    onChange={(e) => setBulkStyle(e.target.value)}
                    disabled={bulkApplying}
                    className="h-7 max-w-[165px] rounded-lg bg-surface-elevated px-2 text-[11px] text-fg"
                  >
                    {presetsQuery.data.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.label}
                      </option>
                    ))}
                  </select>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={bulkApplying}
                    onClick={() => void handleBulkSubtitleStyle(bulkStyle)}
                    className="h-7 px-2.5 text-[11px]"
                  >
                    Appliquer
                  </Button>
                </div>
              )}

              <div className="flex flex-wrap items-center gap-1 rounded-xl border border-border/70 bg-surface-base/60 p-1">
                <span className="px-1.5 text-[11px] text-fg-muted">Texte :</span>
                <Button size="sm" variant="secondary" disabled={bulkApplying} onClick={() => void handleBulkOverlays({ hide_subtitles: true }, "Sans sous-titres")} className="h-7 px-2 text-[11px]">
                  Sans ST
                </Button>
                <Button size="sm" variant="secondary" disabled={bulkApplying} onClick={() => void handleBulkOverlays({ hide_subtitles: false }, "Sous-titres activés")} className="h-7 px-2 text-[11px]">
                  Avec ST
                </Button>
                <Button size="sm" variant="secondary" disabled={bulkApplying} onClick={() => void handleBulkOverlays({ hide_hook: true }, "Sans accroche")} className="h-7 px-2 text-[11px]">
                  Sans hook
                </Button>
                <Button size="sm" variant="secondary" disabled={bulkApplying} onClick={() => void handleBulkOverlays({ hide_hook: false }, "Accroche activée")} className="h-7 px-2 text-[11px]">
                  Avec hook
                </Button>
                <Button size="sm" variant="outline" disabled={bulkApplying} onClick={() => void handleBulkOverlays({ hide_subtitles: true, hide_hook: true }, "Vidéo brute (sans texte)")} className="h-7 px-2 text-[11px]">
                  Vidéo brute
                </Button>
              </div>

              <div className="flex items-center gap-1.5">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={bulkApplying}
                  onClick={() => void handleBulkReviewStatus("approved")}
                  className="h-8 border-status-success/40 text-status-success hover:bg-status-success/10"
                >
                  <Check className="h-3.5 w-3.5" />
                  Valider
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={bulkApplying}
                  onClick={() => void handleBulkReviewStatus("rejected")}
                  className="h-8 border-status-danger/40 text-status-danger hover:bg-status-danger/10"
                >
                  <X className="h-3.5 w-3.5" />
                  Rejeter
                </Button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Grille des clips */}
      {allClips.length === 0 ? (
        <EmptyState
          icon={Film}
          title="Aucun clip prêt pour la revue"
          description={
            (timelineQuery.data?.candidates.length ?? 0) > 0
              ? `${timelineQuery.data?.candidates.length} candidats locaux ont été détectés. Promouvez-les en clips pour les prévisualiser, éditer et exporter en 9:16 (0 token).`
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
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
          {filteredClips.map((clip) => {
            const isSelected = selectedClip?.id === clip.id;
            const isBulkChecked = bulkSelectedSet.has(clip.id);
            const dur = Math.max(1, Math.round(clip.end_sec - clip.start_sec));
            const cardPrev = previewCache[clip.id];
            const cardSrc = cardPrev?.preview_url ?? null;
            const isPlayingThis = playingCardId === clip.id;
            const isLoadingThis = cardLoadingId === clip.id;
            const ov = parseClipOverrides(clip);

            return (
              <motion.div
                key={clip.id}
                whileHover={{ y: -3 }}
                transition={{ duration: 0.14 }}
                onClick={(e) => {
                  if (e.ctrlKey || e.metaKey) {
                    toggleBulkSelect(clip.id);
                  } else {
                    setSelectedClipId(clip.id);
                  }
                }}
                className={`group cursor-pointer overflow-hidden rounded-2xl border p-3 transition-colors cf-glass ${
                  isBulkChecked
                    ? "border-accent-violet ring-2 ring-accent-violet/45"
                    : isSelected
                      ? "border-accent-cyan ring-1 ring-accent-cyan/40"
                      : "border-border/75 hover:border-border-strong"
                }`}
              >
                {/* Carte verticale 9:16 avec lecteur vidéo HTTP Range intégré */}
                <div className="relative flex aspect-[9/16] w-full flex-col justify-between overflow-hidden rounded-xl border border-border/60 bg-gradient-to-b from-surface-base via-surface-elevated to-surface-base p-3">
                  {cardSrc && cardPrev && (
                    <video
                      ref={(el) => {
                        cardVideoRefs.current[clip.id] = el;
                      }}
                      src={cardSrc}
                      autoPlay={isPlayingThis}
                      loop
                      muted={cardMuted}
                      playsInline
                      preload="auto"
                      onPlay={() => setPlayingCardId(clip.id)}
                      onPause={() => {
                        if (playingCardId === clip.id) setPlayingCardId(null);
                      }}
                      onError={(e) => {
                        const code = e.currentTarget.error?.code;
                        toast.error(`Erreur lecteur vidéo (code ${code ?? "inconnu"}) sur « ${clip.title} ».`);
                        setPlayingCardId(null);
                      }}
                      onClick={(e) => {
                        e.stopPropagation();
                        void handleToggleCardPreview(clip);
                      }}
                      className={`absolute inset-0 h-full w-full ${
                        cardPrev.is_rendered || clip.layout_mode === "fill"
                          ? "object-cover"
                          : "object-contain bg-black"
                      }`}
                      style={
                        !cardPrev.is_rendered && clip.layout_mode === "fill"
                          ? { objectPosition: `${Math.round((ov.pan_x + 1) * 50)}% 50%` }
                          : undefined
                      }
                    />
                  )}

                  {cardSrc && (
                    <div className="pointer-events-none absolute inset-0 bg-gradient-to-b from-black/50 via-transparent to-black/70" />
                  )}

                  {/* Top bar */}
                  <div className="relative z-10 flex items-center justify-between gap-1.5">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <button
                        type="button"
                        aria-label={
                          isBulkChecked ? "Désélectionner ce clip" : "Sélectionner ce clip"
                        }
                        onClick={(e) => {
                          e.stopPropagation();
                          toggleBulkSelect(clip.id);
                        }}
                        className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-lg border transition-colors ${
                          isBulkChecked
                            ? "border-accent-cyan bg-accent-violet text-white"
                            : "border-border-strong/70 bg-surface-base/75 text-fg-muted hover:border-accent-cyan hover:text-fg"
                        }`}
                      >
                        {isBulkChecked ? (
                          <Check className="h-3.5 w-3.5" />
                        ) : (
                          <span className="h-2.5 w-2.5 rounded-sm border border-current opacity-50" />
                        )}
                      </button>
                      <Badge variant="cyan" className="truncate uppercase text-[10px]">
                        {clip.category}
                      </Badge>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      {cardSrc && (
                        <>
                          <button
                            type="button"
                            title={isPlayingThis ? "Pause" : "Lire"}
                            onClick={(e) => {
                              e.stopPropagation();
                              void handleToggleCardPreview(clip);
                            }}
                            className="flex h-6 w-6 items-center justify-center rounded-lg bg-black/65 text-white hover:bg-black/85"
                          >
                            {isPlayingThis ? (
                              <Pause className="h-3.5 w-3.5" />
                            ) : (
                              <Play className="h-3.5 w-3.5 fill-current" />
                            )}
                          </button>
                          <button
                            type="button"
                            title={cardMuted ? "Activer le son" : "Couper le son"}
                            onClick={(e) => {
                              e.stopPropagation();
                              setCardMuted((m) => !m);
                            }}
                            className="flex h-6 w-6 items-center justify-center rounded-lg bg-black/65 text-white hover:bg-black/85"
                          >
                            {cardMuted ? (
                              <VolumeX className="h-3.5 w-3.5" />
                            ) : (
                              <Volume2 className="h-3.5 w-3.5" />
                            )}
                          </button>
                        </>
                      )}
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

                  {/* Centre : accroche + bouton Prévisualiser */}
                  <div className="relative z-10 my-auto flex flex-col items-center gap-2.5 text-center">
                    {!cardPrev?.is_rendered && !ov.hide_hook && clip.hook && (
                      <div className="mx-auto max-w-[92%] rounded-lg bg-accent-violet/85 px-2.5 py-1.5 text-[11px] font-bold uppercase tracking-wide text-white shadow">
                        {clip.hook}
                      </div>
                    )}

                    <button
                      type="button"
                      disabled={isLoadingThis}
                      onClick={(e) => {
                        e.stopPropagation();
                        void handleToggleCardPreview(clip);
                      }}
                      className={`inline-flex items-center gap-1.5 rounded-full border px-3.5 py-2 text-xs font-semibold shadow-lg transition-all ${
                        isPlayingThis
                          ? "border-white/25 bg-black/45 text-white opacity-0 group-hover:opacity-100"
                          : "border-accent-cyan/60 bg-slate-950/80 text-white hover:scale-105 hover:bg-accent-violet/90"
                      }`}
                    >
                      {isLoadingThis ? (
                        <>
                          <Loader2 className="h-4 w-4 animate-spin text-accent-cyan" />
                          Préparation…
                        </>
                      ) : isPlayingThis ? (
                        <>
                          <Pause className="h-4 w-4" />
                          Pause
                        </>
                      ) : (
                        <>
                          <Play className="h-4 w-4 fill-current text-accent-cyan" />
                          {cardSrc ? "Lire l'aperçu" : "Prévisualiser"}
                        </>
                      )}
                    </button>

                    {!cardSrc && (
                      <div className="rounded-md border border-dashed border-border-strong/70 bg-surface-subtle/40 px-2 py-1 text-[10px] uppercase tracking-wider text-fg-subtle">
                        {clip.layout_mode} · 9:16
                      </div>
                    )}
                  </div>

                  {/* Pied de carte 9:16 */}
                  <div className="relative z-10 space-y-1.5">
                    <p className="line-clamp-2 text-xs font-semibold leading-snug text-white drop-shadow">
                      {clip.title || "Sans titre"}
                    </p>
                    <div className="flex items-center justify-between gap-1 font-mono text-[10px] text-slate-200/90">
                      <span className="truncate">
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
                        className="shrink-0 text-[10px]"
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
                <div className="mt-2.5 grid grid-cols-3 gap-1">
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 w-full min-w-0 gap-1 px-1.5 text-[11px] text-status-danger hover:bg-status-danger/10"
                    onClick={(e) => {
                      e.stopPropagation();
                      void handleReviewStatus(clip, "rejected");
                    }}
                  >
                    <X className="h-3.5 w-3.5 shrink-0" />
                    <span className="truncate">Rejeter</span>
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-8 w-full min-w-0 gap-1 px-1.5 text-[11px]"
                    onClick={(e) => {
                      e.stopPropagation();
                      setSelectedClipId(clip.id);
                      loadClipIntoDrawer(clip);
                    }}
                  >
                    <Edit3 className="h-3.5 w-3.5 shrink-0" />
                    <span className="truncate">Éditer</span>
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 w-full min-w-0 gap-1 px-1.5 text-[11px] text-status-success hover:bg-status-success/10"
                    onClick={(e) => {
                      e.stopPropagation();
                      void handleReviewStatus(clip, "approved");
                    }}
                  >
                    <Check className="h-3.5 w-3.5 shrink-0" />
                    <span className="truncate">Valider</span>
                  </Button>
                </div>

                <Button
                  size="sm"
                  variant="secondary"
                  className="mt-1.5 h-8 w-full gap-1.5 text-[11px]"
                  onClick={(e) => {
                    e.stopPropagation();
                    openTikTokModalForClip(clip);
                  }}
                >
                  <Upload className="h-3.5 w-3.5 text-accent-cyan" />
                  Publier sur TikTok
                </Button>
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
              <div className="flex items-center justify-between border-b border-border/70 px-5 py-3.5">
                <div className="min-w-0">
                  <h2 className="truncate text-base font-semibold text-fg">
                    Éditeur de clip · {formatDuration(editStart)} → {formatDuration(editEnd)} (
                    {Math.max(1, Math.round(editEnd - editStart))} s)
                  </h2>
                  <p className="text-xs text-fg-muted">
                    Aperçu vidéo 9:16 en boucle, découpe, mise en page, sous-titres et accroche.
                  </p>
                </div>
                <Button size="icon" variant="ghost" onClick={() => setDrawerOpen(false)}>
                  <X className="h-4 w-4" />
                </Button>
              </div>

              <div className="grid flex-1 grid-cols-1 gap-5 overflow-y-auto p-5 md:grid-cols-5">
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
                    {videoAssetUrl && videoPreview ? (
                      <video
                        ref={videoRef}
                        src={videoAssetUrl}
                        autoPlay
                        loop
                        playsInline
                        preload="auto"
                        onPlay={() => setIsLoopPlaying(true)}
                        onPause={() => setIsLoopPlaying(false)}
                        onTimeUpdate={handleDrawerVideoTimeUpdate}
                        onError={(e) => {
                          toast.error(`Erreur lecteur vidéo (code ${e.currentTarget.error?.code ?? "inconnu"}).`);
                        }}
                        onClick={toggleLoopPlay}
                        className={`h-full w-full cursor-pointer ${
                          videoPreview.is_rendered || editLayout === "fill"
                            ? "object-cover"
                            : "object-contain bg-black"
                        }`}
                        style={
                          !videoPreview.is_rendered && editLayout === "fill"
                            ? { objectPosition: `${Math.round((editOverrides.pan_x + 1) * 50)}% 50%` }
                            : undefined
                        }
                      />
                    ) : (
                      <>
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
                      </>
                    )}

                    {!videoPreview?.is_rendered && !editOverrides.hide_hook && editHook.trim() && (
                      <div className="pointer-events-none absolute left-3 right-3 top-[12%] rounded-lg bg-slate-950/85 px-2.5 py-1.5 text-center text-[11px] font-bold uppercase text-white">
                        {editHook}
                      </div>
                    )}

                    {!videoPreview?.is_rendered && !editOverrides.hide_subtitles && (
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
                    )}

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

                  <Card className="space-y-2.5 p-3">
                    <div className="flex flex-wrap items-center justify-between gap-1.5">
                      <div className="flex items-center gap-1.5">
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={toggleLoopPlay}
                          disabled={!audioAssetUrl && !videoAssetUrl}
                        >
                          {isLoopPlaying ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
                          {isLoopPlaying ? "Pause" : "Lire"}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={loadingPreview}
                          onClick={() => void handleLoadDrawerVideoPreview(selectedClip)}
                        >
                          {loadingPreview ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Video className="h-3.5 w-3.5 text-accent-cyan" />
                          )}
                          {videoPreview ? "Recharger" : "Aperçu vidéo"}
                        </Button>
                      </div>
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
                    <div className="space-y-2 text-xs">
                      <div className="space-y-1">
                        <label className="flex justify-between text-fg-muted">
                          <span>Début</span>
                          <span className="font-mono text-fg">
                            {formatDuration(editStart)} ({editStart.toFixed(1)} s)
                          </span>
                        </label>
                        <input
                          type="range"
                          min={Math.max(0, Math.min(editStart, trimAnchorStart) - 90)}
                          max={Math.max(1, editEnd - 2)}
                          step={0.5}
                          value={editStart}
                          onChange={(e) => setEditStart(Number(e.target.value))}
                          className="w-full accent-violet-500"
                        />
                        <div className="grid grid-cols-6 gap-1">
                          {([-10, -5, -1, 1, 5, 10] as const).map((delta) => (
                            <button
                              key={delta}
                              type="button"
                              onClick={() =>
                                setEditStart((prev) =>
                                  Number(Math.min(editEnd - 2, Math.max(0, prev + delta)).toFixed(1)),
                                )
                              }
                              className="rounded-md border border-border/70 bg-surface-subtle/60 py-1 font-mono text-[10px] text-fg-muted hover:border-accent-violet hover:text-fg"
                            >
                              {delta > 0 ? `+${delta}s` : `${delta}s`}
                            </button>
                          ))}
                        </div>
                      </div>

                      <div className="space-y-1 pt-1">
                        <label className="flex justify-between text-fg-muted">
                          <span>Fin (prolonger la chute)</span>
                          <span className="font-mono text-fg">
                            {formatDuration(editEnd)} ({editEnd.toFixed(1)} s)
                          </span>
                        </label>
                        <input
                          type="range"
                          min={editStart + 2}
                          max={Math.min(maxVodDur, Math.max(editEnd, trimAnchorEnd) + 90)}
                          step={0.5}
                          value={editEnd}
                          onChange={(e) => setEditEnd(Number(e.target.value))}
                          className="w-full accent-cyan-400"
                        />
                        <div className="grid grid-cols-6 gap-1">
                          {([-10, -5, -1, 1, 5, 10] as const).map((delta) => (
                            <button
                              key={delta}
                              type="button"
                              onClick={() =>
                                setEditEnd((prev) =>
                                  Number(
                                    Math.min(maxVodDur, Math.max(editStart + 2, prev + delta)).toFixed(1),
                                  ),
                                )
                              }
                              className="rounded-md border border-border/70 bg-surface-subtle/60 py-1 font-mono text-[10px] text-fg-muted hover:border-accent-cyan hover:text-fg"
                            >
                              {delta > 0 ? `+${delta}s` : `${delta}s`}
                            </button>
                          ))}
                        </div>
                      </div>
                    </div>
                  </Card>
                </div>

                <div className="space-y-4 md:col-span-3">
                  <div className="space-y-2.5 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex flex-col gap-1">
                        <span className="text-xs font-semibold text-fg">Accroche, titre & légende</span>
                        <label className="inline-flex cursor-pointer items-center gap-1.5 text-[11px] text-fg-muted">
                          <input
                            type="checkbox"
                            checked={!editOverrides.hide_hook}
                            onChange={(e) =>
                              setEditOverrides({ ...editOverrides, hide_hook: !e.target.checked })
                            }
                            className="accent-violet-500"
                          />
                          Afficher l'accroche sur la vidéo
                        </label>
                      </div>
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

                  <div className="space-y-3 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                    <div className="flex items-center gap-2 text-xs font-semibold text-fg">
                      <TypeIcon className="h-3.5 w-3.5 text-accent-cyan" />
                      Style des sous-titres mot à mot
                    </div>
                    <label className="inline-flex cursor-pointer items-center gap-2 text-xs text-fg-muted">
                      <input
                        type="checkbox"
                        checked={!editOverrides.hide_subtitles}
                        onChange={(e) =>
                          setEditOverrides({ ...editOverrides, hide_subtitles: !e.target.checked })
                        }
                        className="accent-cyan-400"
                      />
                      Afficher les sous-titres mot à mot sur la vidéo
                    </label>
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

              <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border/70 px-5 py-3">
                <Button variant="outline" onClick={() => openTikTokModalForClip(selectedClip)}>
                  <Upload className="h-3.5 w-3.5 text-accent-cyan" />
                  Publier sur TikTok
                </Button>
                <div className="flex items-center gap-2">
                  <Button variant="secondary" disabled={saving} onClick={() => void handleSaveEdits(false)}>
                    <Save className="h-3.5 w-3.5" />
                    Enregistrer
                  </Button>
                  <Button variant="primary" disabled={saving} onClick={() => void handleSaveEdits(true)}>
                    <Check className="h-3.5 w-3.5" />
                    Enregistrer & valider
                  </Button>
                </div>
              </div>
            </motion.aside>
          </>
        )}
      </AnimatePresence>

      {/* Modal TikTok parfaitement centrée (flexbox overlay, aucun conflit transform) */}
      <AnimatePresence>
        {tiktokModalOpen && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={() => setTiktokModalOpen(false)}
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm"
          >
            <motion.div
              initial={{ opacity: 0, scale: 0.96, y: 12 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.96, y: 12 }}
              transition={{ duration: 0.15 }}
              onClick={(e) => e.stopPropagation()}
              className="flex max-h-[86vh] w-full max-w-[590px] flex-col overflow-hidden rounded-2xl border border-border bg-surface-base shadow-2xl"
            >
              {/* En-tête modal */}
              <div className="flex items-center justify-between border-b border-border/70 px-5 py-3.5">
                <div className="min-w-0">
                  <h2 className="flex items-center gap-2 text-base font-semibold text-fg">
                    <Upload className="h-4 w-4 text-accent-cyan" />
                    Publier sur TikTok
                  </h2>
                  <p className="truncate text-xs text-fg-muted">
                    {tiktokClip ? tiktokClip.title : "Lier votre compte TikTok à ClipForge"}
                  </p>
                </div>
                <Button size="icon" variant="ghost" onClick={() => setTiktokModalOpen(false)}>
                  <X className="h-4 w-4" />
                </Button>
              </div>

              {/* Sélecteur de mode : Intégré (0 clé) vs API OAuth2 */}
              <div className="grid grid-cols-2 gap-2 border-b border-border/60 bg-surface-subtle/20 px-5 py-2.5">
                <Button
                  size="sm"
                  variant={tiktokTab === "integrated" ? "primary" : "secondary"}
                  onClick={() => setTiktokTab("integrated")}
                >
                  <Link2 className="h-3.5 w-3.5" />
                  Compte intégré (0 clé)
                </Button>
                <Button
                  size="sm"
                  variant={tiktokTab === "api" ? "primary" : "secondary"}
                  onClick={() => setTiktokTab("api")}
                >
                  <KeyRound className="h-3.5 w-3.5" />
                  API OAuth2 (Avancé)
                </Button>
              </div>

              {/* Corps scrollable */}
              <div className="space-y-4 overflow-y-auto p-5 text-xs">
                {tiktokTab === "integrated" ? (
                  <>
                    {/* Étape 1 : Connexion unique (0 clé, 0 €) */}
                    <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-border/70 bg-surface-subtle/30 p-3">
                      <p className="min-w-0 flex-1 text-[11px] leading-relaxed text-fg-muted">
                        <strong className="text-fg">1. Compte TikTok intégré (0 clé)</strong> : connecte-toi une
                        seule fois (session sauvegardée), puis ClipForge injecte automatiquement le{" "}
                        <strong className="text-fg">MP4 + description + #</strong>.
                      </p>
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={ttOpeningStudio}
                        onClick={() => void handleOpenIntegratedTikTokWindow()}
                      >
                        {ttOpeningStudio ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Link2 className="h-3.5 w-3.5 text-accent-cyan" />
                        )}
                        Lier / Ouvrir TikTok
                      </Button>
                    </div>

                    {tiktokClip && (
                      <div className="space-y-3.5 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                        {/* Étape 2 : Description IA modifiable + 3 variantes */}
                        <div className="space-y-2">
                          <div className="flex flex-wrap items-center justify-between gap-2">
                            <span className="text-[11px] font-semibold text-fg">
                              2. Description recommandée par l'IA
                            </span>
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={ttLoadingAi}
                              onClick={() => void handleSuggestTikTokAi(tiktokClip)}
                              className="h-7 px-2.5 text-[11px]"
                            >
                              {ttLoadingAi ? (
                                <Loader2 className="h-3 w-3 animate-spin" />
                              ) : (
                                <Wand2 className="h-3 w-3 text-accent-cyan" />
                              )}
                              {ttLoadingAi ? "Génération…" : "Proposer 3 variantes IA"}
                            </Button>
                          </div>

                          {ttAiVariants.length > 0 && (
                            <div className="space-y-1.5">
                              {ttAiVariants.map((variant, idx) => {
                                const labels = ["Drôle / Punchy", "Courte", "Question commentaires"];
                                const active = ttDescription.trim() === variant.trim();
                                return (
                                  <button
                                    key={idx}
                                    type="button"
                                    onClick={() => setTtDescription(variant)}
                                    className={`w-full rounded-lg border p-2 text-left text-[11px] transition-colors ${
                                      active
                                        ? "border-accent-cyan bg-accent-cyan/10 text-fg"
                                        : "border-border/70 bg-surface-base/70 text-fg-muted hover:border-border-strong hover:text-fg"
                                    }`}
                                  >
                                    <span className="mb-0.5 block text-[10px] font-semibold uppercase tracking-wide text-accent-cyan">
                                      {labels[idx] ?? `Variante ${idx + 1}`}
                                    </span>
                                    {variant}
                                  </button>
                                );
                              })}
                            </div>
                          )}

                          <textarea
                            rows={2}
                            value={ttDescription}
                            onChange={(e) => setTtDescription(e.target.value)}
                            placeholder="Écris ta description ou choisis une variante IA ci-dessus…"
                            className="w-full rounded-lg border border-border bg-surface-base p-2.5 text-xs text-fg"
                          />
                        </div>

                        {/* Étape 3 : Puces #hashtags cliquables + favoris mémorisés */}
                        <div className="space-y-2 border-t border-border/50 pt-3">
                          <div className="flex items-center justify-between">
                            <span className="flex items-center gap-1 text-[11px] font-semibold text-fg">
                              <Hash className="h-3 w-3 text-accent-cyan" />
                              3. Hashtags ({ttActiveTags.length} actif(s))
                            </span>
                            <span className="text-[10px] text-fg-subtle">
                              Clic = activer · ★ = mémoriser
                            </span>
                          </div>

                          <div className="flex flex-wrap gap-1.5">
                            {ttSuggestedTags.map((tag) => {
                              const isActive = ttActiveTags.includes(tag);
                              const isFav = ttFavTags.includes(tag);
                              return (
                                <div
                                  key={tag}
                                  className={`inline-flex items-center overflow-hidden rounded-full border text-[11px] transition-colors ${
                                    isActive
                                      ? "border-accent-cyan bg-accent-cyan/15 text-fg"
                                      : "border-border/70 bg-surface-base/70 text-fg-muted hover:border-border-strong"
                                  }`}
                                >
                                  <button
                                    type="button"
                                    onClick={() => toggleTikTokTag(tag)}
                                    className="px-2.5 py-1 font-mono"
                                  >
                                    #{tag}
                                  </button>
                                  <button
                                    type="button"
                                    title={isFav ? "Retirer des favoris" : "Mémoriser pour tous mes clips"}
                                    onClick={() => toggleFavTikTokTag(tag)}
                                    className={`border-l border-border/50 px-1.5 py-1 hover:bg-surface-subtle/60 ${
                                      isFav ? "text-amber-400" : "text-fg-subtle"
                                    }`}
                                  >
                                    <Star className={`h-3 w-3 ${isFav ? "fill-current" : ""}`} />
                                  </button>
                                </div>
                              );
                            })}
                          </div>

                          <div className="flex gap-1.5">
                            <input
                              value={ttCustomTag}
                              onChange={(e) => setTtCustomTag(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                  e.preventDefault();
                                  handleAddCustomTikTokTag();
                                }
                              }}
                              placeholder="Ajouter tes hashtags (ex. gta6, rp, moro)…"
                              className="h-8 flex-1 rounded-lg border border-border bg-surface-base px-2.5 font-mono text-[11px] text-fg"
                            />
                            <Button
                              size="sm"
                              variant="secondary"
                              onClick={handleAddCustomTikTokTag}
                              className="h-8 px-2.5"
                            >
                              <Plus className="h-3.5 w-3.5" />
                              Ajouter
                            </Button>
                          </div>
                        </div>

                        {/* Aperçu final + mode publication */}
                        <div className="space-y-2 border-t border-border/50 pt-3">
                          <div className="flex items-center justify-between text-[10px] text-fg-subtle">
                            <span>Texte final qui sera injecté sur TikTok :</span>
                            <span className="font-mono">{ttFinalCaption.length} / 2150</span>
                          </div>
                          <div className="max-h-20 overflow-y-auto whitespace-pre-wrap rounded-lg border border-border/60 bg-surface-base/60 p-2 text-[11px] text-fg-muted">
                            {ttFinalCaption || "(Vide)"}
                          </div>

                          <label className="inline-flex cursor-pointer items-center gap-2 text-[11px] text-fg-muted">
                            <input
                              type="checkbox"
                              checked={ttAutoPublish}
                              onChange={(e) => setTtAutoPublish(e.target.checked)}
                              className="accent-cyan-400"
                            />
                            Cliquer automatiquement sur « Publier » une fois la vidéo traitée (sinon laisse 1 clic final)
                          </label>
                        </div>

                        {ttInjectResult && (
                          <div className="rounded-lg border border-status-success/40 bg-status-success/10 p-2.5 text-[11px] text-status-success">
                            <p className="font-semibold">{ttInjectResult.message}</p>
                            <p className="mt-0.5 font-mono opacity-80">
                              {ttInjectResult.file_name} · {(ttInjectResult.file_size_bytes / (1024 * 1024)).toFixed(1)} Mo
                            </p>
                          </div>
                        )}

                        <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => void handleOpenExternalBrowserTikTok(tiktokClip)}
                          >
                            <ExternalLink className="h-3.5 w-3.5" />
                            Via navigateur
                          </Button>
                          <Button
                            size="sm"
                            variant="primary"
                            disabled={ttUploading}
                            onClick={() => void handleIntegratedPublishTikTok(tiktokClip)}
                          >
                            {ttUploading ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <Upload className="h-3.5 w-3.5" />
                            )}
                            {ttUploading
                              ? tiktokClip.output_mp4_path
                                ? "Auto-injection TikTok…"
                                : "Encodage MP4 + Auto-injection…"
                              : ttAutoPublish
                                ? "Uploader & Publier (Auto)"
                                : "Uploader & Pré-remplir (Auto)"}
                          </Button>
                        </div>
                      </div>
                    )}
                  </>
                ) : (
                  <>
                    {/* Onglet API OAuth2 officielle */}
                    <div className="space-y-2.5 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div className="flex items-center gap-2">
                          <Badge variant={ttStatus?.connected ? "success" : "default"}>
                            {ttStatus?.connected
                              ? `Connecté${ttStatus.creator_nickname ? ` · ${ttStatus.creator_nickname}` : ""}`
                              : "Non connecté"}
                          </Badge>
                          {ttStatus?.client_key_preview && (
                            <span className="font-mono text-[11px] text-fg-subtle">
                              Key {ttStatus.client_key_preview}
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-1.5">
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => setTtShowConfig((v) => !v)}
                          >
                            <KeyRound className="h-3.5 w-3.5" />
                            {ttShowConfig ? "Masquer clés" : "Configurer clés"}
                          </Button>
                          {ttStatus?.connected && (
                            <Button size="sm" variant="ghost" onClick={() => void handleDisconnectTikTok()}>
                              <LogOut className="h-3.5 w-3.5 text-status-danger" />
                            </Button>
                          )}
                        </div>
                      </div>

                      {ttShowConfig && (
                        <div className="space-y-2 border-t border-border/50 pt-2.5">
                          <p className="text-[11px] leading-relaxed text-fg-muted">
                            Optionnel (upload silencieux sans fenêtre) : crée une app sur{" "}
                            <span className="font-mono text-fg">developers.tiktok.com</span> avec{" "}
                            <strong className="text-fg">Login Kit</strong> +{" "}
                            <strong className="text-fg">Content Posting API</strong> et Redirect URI{" "}
                            <code className="rounded bg-surface-base px-1 py-0.5 text-[10px] text-accent-cyan">
                              http://127.0.0.1:48321/tiktok/callback
                            </code>
                          </p>
                          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                            <input
                              value={ttClientKey}
                              onChange={(e) => setTtClientKey(e.target.value)}
                              placeholder="Client Key TikTok"
                              className="h-8 rounded-lg border border-border bg-surface-base px-2.5 text-xs text-fg"
                            />
                            <input
                              type="password"
                              value={ttClientSecret}
                              onChange={(e) => setTtClientSecret(e.target.value)}
                              placeholder="Client Secret TikTok"
                              className="h-8 rounded-lg border border-border bg-surface-base px-2.5 text-xs text-fg"
                            />
                          </div>
                          <input
                            value={ttRedirectUri}
                            onChange={(e) => setTtRedirectUri(e.target.value)}
                            placeholder="Redirect URI"
                            className="h-8 w-full rounded-lg border border-border bg-surface-base px-2.5 font-mono text-[11px] text-fg"
                          />
                          <label className="inline-flex cursor-pointer items-center gap-2 text-[11px] text-fg-muted">
                            <input
                              type="checkbox"
                              checked={ttUsePkce}
                              onChange={(e) => setTtUsePkce(e.target.checked)}
                              className="accent-cyan-400"
                            />
                            Activer PKCE S256 (apps Desktop TikTok)
                          </label>
                          <div className="flex flex-wrap items-center gap-2 pt-1">
                            <Button
                              size="sm"
                              variant="secondary"
                              disabled={ttSavingCreds}
                              onClick={() => void handleSaveTikTokCreds()}
                            >
                              {ttSavingCreds ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
                              Enregistrer
                            </Button>
                            <Button
                              size="sm"
                              variant="primary"
                              disabled={ttConnecting || !ttStatus?.client_configured}
                              onClick={() => void handleConnectTikTokOAuth()}
                            >
                              {ttConnecting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Share2 className="h-3.5 w-3.5" />}
                              {ttConnecting ? "En attente…" : "Connecter OAuth2"}
                            </Button>
                          </div>
                        </div>
                      )}

                      {!ttShowConfig && !ttStatus?.connected && ttStatus?.client_configured && (
                        <Button
                          size="sm"
                          variant="primary"
                          disabled={ttConnecting}
                          onClick={() => void handleConnectTikTokOAuth()}
                        >
                          {ttConnecting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Share2 className="h-3.5 w-3.5" />}
                          Connecter mon compte TikTok
                        </Button>
                      )}
                    </div>

                    {tiktokClip && (
                      <div className="space-y-3 rounded-xl border border-border/70 bg-surface-subtle/30 p-3.5">
                        <div className="grid grid-cols-2 gap-2">
                          <Button
                            size="sm"
                            variant={ttUploadMode === "inbox" ? "primary" : "secondary"}
                            onClick={() => setTtUploadMode("inbox")}
                          >
                            Brouillon TikTok
                          </Button>
                          <Button
                            size="sm"
                            variant={ttUploadMode === "direct" ? "primary" : "secondary"}
                            onClick={() => setTtUploadMode("direct")}
                          >
                            Publication directe
                          </Button>
                        </div>

                        {ttUploadMode === "direct" && (
                          <div className="space-y-2">
                            <select
                              value={ttPrivacy}
                              onChange={(e) => setTtPrivacy(e.target.value as TikTokPrivacy)}
                              className="h-9 w-full rounded-lg border border-border bg-surface-elevated px-3 text-xs text-fg"
                            >
                              <option value="SELF_ONLY">Moi uniquement (Privé — sans audit)</option>
                              <option value="MUTUAL_FOLLOW_FRIENDS">Amis</option>
                              <option value="FOLLOWER_OF_CREATOR">Abonnés</option>
                              <option value="PUBLIC_TO_EVERYONE">Public (app TikTok auditée requise)</option>
                            </select>
                            <div className="flex flex-wrap gap-4 text-[11px] text-fg-muted">
                              <label className="inline-flex items-center gap-1.5">
                                <input
                                  type="checkbox"
                                  checked={!ttDisableComment}
                                  onChange={(e) => setTtDisableComment(!e.target.checked)}
                                  className="accent-cyan-400"
                                />
                                Commentaires
                              </label>
                              <label className="inline-flex items-center gap-1.5">
                                <input
                                  type="checkbox"
                                  checked={!ttDisableDuet}
                                  onChange={(e) => setTtDisableDuet(!e.target.checked)}
                                  className="accent-cyan-400"
                                />
                                Duo
                              </label>
                              <label className="inline-flex items-center gap-1.5">
                                <input
                                  type="checkbox"
                                  checked={!ttDisableStitch}
                                  onChange={(e) => setTtDisableStitch(!e.target.checked)}
                                  className="accent-cyan-400"
                                />
                                Collage
                              </label>
                            </div>
                          </div>
                        )}

                        <textarea
                          rows={2}
                          value={ttCaption}
                          onChange={(e) => setTtCaption(e.target.value)}
                          className="w-full rounded-lg border border-border bg-surface-base p-2.5 text-xs text-fg"
                        />

                        {ttLastResult && (
                          <div className="rounded-lg border border-status-success/40 bg-status-success/10 p-2.5 text-[11px] text-status-success">
                            <p className="font-semibold">{ttLastResult.message}</p>
                            <p className="mt-0.5 font-mono opacity-80">ID : {ttLastResult.publish_id}</p>
                          </div>
                        )}

                        <div className="flex justify-end">
                          <Button
                            size="sm"
                            variant="primary"
                            disabled={ttUploading || !ttStatus?.connected}
                            onClick={() => void handleDirectUploadTikTok()}
                          >
                            {ttUploading ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <Upload className="h-3.5 w-3.5" />
                            )}
                            {ttUploading ? "Upload API en cours…" : "Uploader via API TikTok"}
                          </Button>
                        </div>
                      </div>
                    )}
                  </>
                )}
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
