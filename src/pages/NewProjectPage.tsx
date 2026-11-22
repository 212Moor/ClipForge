import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  AlertTriangle,
  Clock,
  Coins,
  Cpu,
  Play,
  Plus,
  Search,
  ShieldAlert,
  Sparkles,
  UserCheck,
} from "lucide-react";
import {
  commands,
  type AnalysisMode,
  type VodProbeMetadata,
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
import { formatCurrency, formatDuration, formatTokens } from "@/lib/format";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

const MODES: { id: AnalysisMode; title: string; desc: string; defaultBudget: number }[] = [
  {
    id: "eco",
    title: "Éco (Batch -50 %)",
    desc: "Étage A seul via Batch API asynchrone (< 60k tokens sur 4 h).",
    defaultBudget: 60_000,
  },
  {
    id: "standard",
    title: "Standard",
    desc: "Cascade Étage A (Flash-Lite) + Étage B (Flash) sur le top K (< 150k tokens).",
    defaultBudget: 150_000,
  },
  {
    id: "quality",
    title: "Qualité",
    desc: "Étage A + Étage B enrichi avec contexte étendu sur les meilleurs clips.",
    defaultBudget: 200_000,
  },
];

export function NewProjectPage() {
  const queryClient = useQueryClient();
  const setActiveScreen = useUiStore((s) => s.setActiveScreen);
  const draftVodUrl = useProjectStore((s) => s.draftVodUrl);
  const setDraftVodUrl = useProjectStore((s) => s.setDraftVodUrl);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);

  const [url, setUrl] = useState(draftVodUrl);
  const [probing, setProbing] = useState(false);
  const [probe, setProbe] = useState<VodProbeMetadata | null>(null);
  const [projectName, setProjectName] = useState("");
  const [mode, setMode] = useState<AnalysisMode>("standard");
  const [targetClips, setTargetClips] = useState(12);
  const [targetMinSec, setTargetMinSec] = useState(30);
  const [targetMaxSec, setTargetMaxSec] = useState(60);
  const [tokenBudgetLimit, setTokenBudgetLimit] = useState(150_000);
  const [asrLanguage, setAsrLanguage] = useState("auto");
  const [profileId, setProfileId] = useState<string>("");
  const [subscriberCookie, setSubscriberCookie] = useState("");
  const [privacyAccepted, setPrivacyAccepted] = useState(false);
  const [launching, setLaunching] = useState(false);

  // Formulaire rapide de profil streamer
  const [showProfileForm, setShowProfileForm] = useState(false);
  const [profName, setProfName] = useState("");
  const [profTone, setProfTone] = useState("Drôle, spontané, énergique");
  const [profGames, setProfGames] = useState("");
  const [profJargon, setProfJargon] = useState("");
  const [profForbidden, setProfForbidden] = useState("Politique, drama, musique DMCA");

  const profilesQuery = useQuery({
    queryKey: ["streamer-profiles"],
    queryFn: async () => {
      const res = await commands.listStreamerProfilesCmd();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const estimateQuery = useQuery({
    queryKey: ["cost-estimate", probe?.duration_sec ?? 14400, targetClips, mode],
    queryFn: async () => {
      const res = await commands.estimateCostBeforeLaunch(
        probe?.duration_sec ?? 14400,
        targetClips,
        mode,
      );
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  useEffect(() => {
    if (draftVodUrl && draftVodUrl !== url) {
      setUrl(draftVodUrl);
    }
  }, [draftVodUrl, url]);

  const handleProbe = async () => {
    if (!url.trim()) {
      toast.error("Collez d'abord une URL ou un ID de VOD Twitch.");
      return;
    }
    setProbing(true);
    try {
      const res = await commands.probeVod(url.trim());
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      setProbe(res.data);
      if (!projectName.trim()) {
        setProjectName(res.data.title);
      }
      toast.success("Métadonnées VOD récupérées via yt-dlp.");
    } finally {
      setProbing(false);
    }
  };

  const handleSaveProfile = async () => {
    if (!profName.trim()) {
      toast.error("Indiquez le nom du streamer.");
      return;
    }
    const res = await commands.saveStreamerProfileCmd({
      id: null,
      name: profName.trim(),
      tone: profTone.trim(),
      jargon: profJargon
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      emotes: {},
      games: profGames
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      inside_jokes: "",
      forbidden_topics: profForbidden.trim(),
    });
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    await queryClient.invalidateQueries({ queryKey: ["streamer-profiles"] });
    setProfileId(res.data.id);
    setShowProfileForm(false);
    toast.success(`Profil « ${res.data.name} » enregistré.`);
  };

  const handleLaunch = async (localOnly: boolean) => {
    if (!url.trim()) {
      toast.error("Renseignez une URL ou un ID de VOD Twitch.");
      return;
    }
    if (!localOnly && !privacyAccepted) {
      toast.warning("Veuillez confirmer l'avertissement d'envoi de texte vers Gemini.");
      return;
    }
    setLaunching(true);
    try {
      const createRes = await commands.createProject({
        name: projectName.trim() || null,
        source_url: url.trim(),
        streamer_profile_id: profileId || null,
        mode,
        target_clips: targetClips,
        target_min_sec: targetMinSec,
        target_max_sec: targetMaxSec,
        token_budget_limit: tokenBudgetLimit,
        asr_language: asrLanguage,
        subscriber_cookie: subscriberCookie.trim() || null,
        probe,
      });
      if (createRes.status === "error") {
        toast.error(createRes.error.message);
        return;
      }
      const projectId = createRes.data;
      setDraftVodUrl("");
      setActiveProjectId(projectId);
      await queryClient.invalidateQueries({ queryKey: ["projects"] });

      const startRes = await commands.startProjectAnalysis(projectId, localOnly);
      if (startRes.status === "error") {
        toast.error(startRes.error.message);
        return;
      }
      toast.success(
        localOnly
          ? "Analyse locale lancée (0 token Gemini)."
          : "Analyse lancée avec plafond strict de tokens.",
      );
      setActiveScreen("analysis");
    } finally {
      setLaunching(false);
    }
  };

  const est = estimateQuery.data;

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-fg">Nouveau projet</h1>
        <p className="text-sm text-fg-muted">
          Prévisualisez la VOD, choisissez votre budget de tokens et lancez le pipeline en moins de 5 clics.
        </p>
      </header>

      {/* 1. Source VOD + aperçu */}
      <Card>
        <CardHeader>
          <CardTitle>1. Source VOD Twitch</CardTitle>
          <CardDescription>
            Seul le flux audio et le chat JSON sont téléchargés pour l'analyse — jamais la vidéo complète.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-col gap-2 sm:flex-row">
            <input
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://www.twitch.tv/videos/1234567890 ou ID numérique"
              className="h-10 flex-1 rounded-xl border border-border bg-surface-subtle/60 px-3.5 text-sm text-fg placeholder:text-fg-subtle focus:border-accent-cyan focus:outline-none"
            />
            <Button variant="secondary" disabled={probing} onClick={() => void handleProbe()}>
              <Search className="h-4 w-4" />
              {probing ? "Lecture yt-dlp…" : "Aperçu VOD"}
            </Button>
          </div>

          {probe && (
            <div className="flex flex-col gap-4 rounded-xl border border-border/70 bg-surface-subtle/40 p-3.5 sm:flex-row">
              {probe.thumbnail_url && (
                <img
                  src={probe.thumbnail_url}
                  alt={probe.title}
                  className="h-24 w-44 shrink-0 rounded-lg object-cover"
                />
              )}
              <div className="min-w-0 space-y-1.5">
                <p className="truncate text-sm font-semibold text-fg">{probe.title}</p>
                <div className="flex flex-wrap items-center gap-2 text-xs text-fg-muted">
                  <Badge variant="violet">{probe.channel_name}</Badge>
                  <Badge variant="default">
                    <Clock className="h-3 w-3" />
                    {formatDuration(probe.duration_sec)}
                  </Badge>
                  {probe.muted_segments.length > 0 && (
                    <Badge variant="warning">
                      <AlertTriangle className="h-3 w-3" />
                      {probe.muted_segments.length} zone(s) mutée(s) exclue(s)
                    </Badge>
                  )}
                </div>
              </div>
            </div>
          )}

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg-muted">Nom du projet (optionnel)</label>
              <input
                type="text"
                value={projectName}
                onChange={(e) => setProjectName(e.target.value)}
                placeholder="Ex. Ranked Valorant — Vendredi soir"
                className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 text-sm text-fg focus:border-accent-cyan focus:outline-none"
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg-muted">Langue transcription (whisper.cpp)</label>
              <select
                value={asrLanguage}
                onChange={(e) => setAsrLanguage(e.target.value)}
                className="h-9 w-full rounded-xl border border-border bg-surface-elevated px-3 text-sm text-fg focus:border-accent-cyan focus:outline-none"
              >
                <option value="auto">Détection automatique</option>
                <option value="fr">Français (fr)</option>
                <option value="en">Anglais (en)</option>
              </select>
            </div>
          </div>

          <details className="rounded-xl border border-border/60 bg-surface-subtle/30 px-3.5 py-2 text-xs">
            <summary className="cursor-pointer font-medium text-fg-muted">
              VOD réservée aux abonnés (cookie Netscape optionnel, stocké uniquement en local)
            </summary>
            <textarea
              rows={3}
              value={subscriberCookie}
              onChange={(e) => setSubscriberCookie(e.target.value)}
              placeholder="# Netscape HTTP Cookie File..."
              className="mt-2 w-full rounded-lg border border-border bg-surface-base/80 p-2 font-mono text-[11px] text-fg focus:outline-none"
            />
          </details>
        </CardContent>
      </Card>

      {/* 2. Mode, paramètres clips & profil streamer */}
      <Card>
        <CardHeader>
          <CardTitle>2. Mode d'analyse & Profil streamer</CardTitle>
          <CardDescription>
            Ajustez le nombre de clips voulus, la durée cible et le profil éditorial injecté dans le préfixe mis en cache.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            {MODES.map((m) => (
              <button
                key={m.id}
                type="button"
                onClick={() => {
                  setMode(m.id);
                  setTokenBudgetLimit(m.defaultBudget);
                }}
                className={`rounded-xl border p-3.5 text-left transition-all ${
                  mode === m.id
                    ? "border-accent-cyan bg-accent-cyan/10"
                    : "border-border/70 bg-surface-subtle/35 hover:border-border-strong"
                }`}
              >
                <p className="text-sm font-semibold text-fg">{m.title}</p>
                <p className="mt-1 text-xs leading-relaxed text-fg-muted">{m.desc}</p>
              </button>
            ))}
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-4">
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg-muted">Clips voulus ({targetClips})</label>
              <input
                type="range"
                min={5}
                max={20}
                value={targetClips}
                onChange={(e) => setTargetClips(Number(e.target.value))}
                className="w-full accent-violet-500"
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg-muted">Durée min (s)</label>
              <input
                type="number"
                min={20}
                max={60}
                value={targetMinSec}
                onChange={(e) => setTargetMinSec(Number(e.target.value))}
                className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 text-sm text-fg"
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg-muted">Durée max (s)</label>
              <input
                type="number"
                min={25}
                max={75}
                value={targetMaxSec}
                onChange={(e) => setTargetMaxSec(Number(e.target.value))}
                className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 text-sm text-fg"
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs font-medium text-fg-muted">Plafond dur (tokens)</label>
              <input
                type="number"
                min={10000}
                step={10000}
                value={tokenBudgetLimit}
                onChange={(e) => setTokenBudgetLimit(Number(e.target.value))}
                className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 font-mono text-sm text-fg"
              />
            </div>
          </div>

          {/* Profil streamer */}
          <div className="space-y-2 border-t border-border/60 pt-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <label className="flex items-center gap-2 text-xs font-medium text-fg-muted">
                <UserCheck className="h-3.5 w-3.5 text-accent-violet" />
                Profil du streamer (saisi une fois, réutilisé en cache)
              </label>
              <Button
                size="sm"
                variant="outline"
                onClick={() => setShowProfileForm((v) => !v)}
              >
                <Plus className="h-3.5 w-3.5" />
                {showProfileForm ? "Fermer" : "Nouveau profil"}
              </Button>
            </div>

            <select
              value={profileId}
              onChange={(e) => setProfileId(e.target.value)}
              className="h-9 w-full rounded-xl border border-border bg-surface-elevated px-3 text-sm text-fg"
            >
              <option value="">Profil générique par défaut</option>
              {profilesQuery.data?.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} — {p.tone}
                </option>
              ))}
            </select>

            {showProfileForm && (
              <div className="grid grid-cols-1 gap-2.5 rounded-xl border border-accent-violet/35 bg-surface-subtle/40 p-3.5 sm:grid-cols-2">
                <input
                  value={profName}
                  onChange={(e) => setProfName(e.target.value)}
                  placeholder="Pseudo du streamer *"
                  className="h-9 rounded-lg border border-border bg-surface-base/80 px-3 text-xs text-fg"
                />
                <input
                  value={profTone}
                  onChange={(e) => setProfTone(e.target.value)}
                  placeholder="Ton (ex. second degré, tryhard, chill)"
                  className="h-9 rounded-lg border border-border bg-surface-base/80 px-3 text-xs text-fg"
                />
                <input
                  value={profGames}
                  onChange={(e) => setProfGames(e.target.value)}
                  placeholder="Jeux (séparés par virgules)"
                  className="h-9 rounded-lg border border-border bg-surface-base/80 px-3 text-xs text-fg"
                />
                <input
                  value={profJargon}
                  onChange={(e) => setProfJargon(e.target.value)}
                  placeholder="Jargon / private jokes (virgules)"
                  className="h-9 rounded-lg border border-border bg-surface-base/80 px-3 text-xs text-fg"
                />
                <input
                  value={profForbidden}
                  onChange={(e) => setProfForbidden(e.target.value)}
                  placeholder="À ne jamais clipper"
                  className="h-9 rounded-lg border border-border bg-surface-base/80 px-3 text-xs text-fg sm:col-span-2"
                />
                <div className="sm:col-span-2 flex justify-end">
                  <Button size="sm" variant="primary" onClick={() => void handleSaveProfile()}>
                    Enregistrer le profil
                  </Button>
                </div>
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      {/* 3. Estimation de coût + avertissement confidentialité + lancement */}
      <Card className="border-accent-cyan/35">
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Coins className="h-4 w-4 text-accent-cyan" />
              <CardTitle>3. Estimation avant lancement</CardTitle>
            </div>
            {est && (
              <Badge variant={est.within_target_budget ? "success" : "warning"}>
                {est.within_target_budget ? "Dans la cible économe" : "Au-dessus de la cible"}
              </Badge>
            )}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {est && (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              <div className="rounded-xl border border-border/60 bg-surface-subtle/40 p-3">
                <p className="text-[11px] text-fg-subtle">Candidats pré-filtrés</p>
                <p className="mt-0.5 font-mono text-base font-semibold text-fg">
                  ≈ {est.estimated_candidates}
                </p>
              </div>
              <div className="rounded-xl border border-border/60 bg-surface-subtle/40 p-3">
                <p className="text-[11px] text-fg-subtle">Tokens estimés (in / out)</p>
                <p className="mt-0.5 font-mono text-base font-semibold text-fg">
                  ≈ {formatTokens(est.estimated_input_tokens)} / {formatTokens(est.estimated_output_tokens)}
                </p>
              </div>
              <div className="rounded-xl border border-border/60 bg-surface-subtle/40 p-3">
                <p className="text-[11px] text-fg-subtle">Coût estimé</p>
                <p className="mt-0.5 font-mono text-base font-semibold text-accent-cyan">
                  ≈ {formatCurrency(est.estimated_cost_eur, "EUR")}
                </p>
              </div>
              <div className="rounded-xl border border-border/60 bg-surface-subtle/40 p-3">
                <p className="text-[11px] text-fg-subtle">Modèles cascade</p>
                <p className="mt-0.5 truncate font-mono text-xs text-fg">
                  A: {est.stage_a_model}
                  {est.stage_b_model ? ` · B: ${est.stage_b_model}` : " (Éco)"}
                </p>
              </div>
            </div>
          )}

          {/* Avertissement confidentialité obligatoire avant envoi vers Gemini */}
          <label className="flex cursor-pointer items-start gap-3 rounded-xl border border-status-warning/40 bg-status-warning/10 p-3.5 text-xs leading-relaxed text-fg">
            <input
              type="checkbox"
              checked={privacyAccepted}
              onChange={(e) => setPrivacyAccepted(e.target.checked)}
              className="mt-0.5 h-4 w-4 accent-cyan-400"
            />
            <span className="space-y-0.5">
              <span className="flex items-center gap-1.5 font-semibold">
                <ShieldAlert className="h-3.5 w-3.5 text-status-warning" />
                Envoi des transcriptions et extraits de chat vers Google Gemini
              </span>
              <span className="block text-fg-muted">
                Seul du texte compact (~10 % de la VOD, aucun audio ni vidéo) est envoyé à l'API Google AI Studio.
                Attention : avec une clé gratuite (Free Tier), Google peut utiliser les requêtes pour améliorer ses produits ; utilisez un projet facturé à l'usage (Paid Tier) pour des données confidentielles.
              </span>
            </span>
          </label>

          <div className="flex flex-wrap items-center justify-end gap-2.5">
            <Button
              variant="secondary"
              disabled={launching}
              onClick={() => void handleLaunch(true)}
            >
              <Cpu className="h-4 w-4" />
              Analyse locale seule (0 token)
            </Button>
            <Button
              variant="primary"
              disabled={launching}
              onClick={() => void handleLaunch(false)}
            >
              <Play className="h-4 w-4" />
              <Sparkles className="h-3.5 w-3.5" />
              {launching ? "Démarrage…" : "Lancer l'analyse complète"}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
