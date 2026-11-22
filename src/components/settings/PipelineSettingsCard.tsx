import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { CheckCircle2, Download, Gauge, Mic, Sliders } from "lucide-react";
import { commands, type RateLimitConfig, type SignalWeightsConfig } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

export function PipelineSettingsCard() {
  const queryClient = useQueryClient();
  const [downloadingModel, setDownloadingModel] = useState<string | null>(null);
  const [weights, setWeights] = useState<SignalWeightsConfig | null>(null);
  const [rateLimit, setRateLimit] = useState<RateLimitConfig | null>(null);

  const whisperModelsQuery = useQuery({
    queryKey: ["whisper-models"],
    queryFn: async () => {
      const res = await commands.listWhisperModels();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const selectedModelQuery = useQuery({
    queryKey: ["selected-whisper-model"],
    queryFn: async () => {
      const res = await commands.getSelectedWhisperModel();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const weightsQuery = useQuery({
    queryKey: ["signal-weights"],
    queryFn: async () => {
      const res = await commands.getSignalWeights();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const rateLimitQuery = useQuery({
    queryKey: ["rate-limit-config"],
    queryFn: async () => {
      const res = await commands.getRateLimitConfig();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  useEffect(() => {
    if (weightsQuery.data) setWeights(weightsQuery.data);
  }, [weightsQuery.data]);

  useEffect(() => {
    if (rateLimitQuery.data) setRateLimit(rateLimitQuery.data);
  }, [rateLimitQuery.data]);

  const handleSelectWhisper = async (id: string) => {
    const res = await commands.setSelectedWhisperModel(id);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success(`Modèle Whisper actif : ${id}`);
    await queryClient.invalidateQueries({ queryKey: ["selected-whisper-model"] });
  };

  const handleDownloadWhisper = async (id: string) => {
    setDownloadingModel(id);
    try {
      const res = await commands.downloadWhisperModel(id);
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      toast.success(`Modèle Whisper « ${id} » téléchargé.`);
      await queryClient.invalidateQueries({ queryKey: ["whisper-models"] });
    } finally {
      setDownloadingModel(null);
    }
  };

  const handleSaveWeights = async () => {
    if (!weights) return;
    const res = await commands.setSignalWeights(weights);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success("Pondérations et liste d'emotes enregistrées.");
  };

  const handleSaveRateLimit = async () => {
    if (!rateLimit) return;
    const res = await commands.setRateLimitConfig(rateLimit);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success("Limiteur RPM/TPM Gemini mis à jour.");
  };

  return (
    <div className="space-y-6">
      {/* Modèles Whisper à la demande */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Mic className="h-4 w-4 text-accent-cyan" />
            <CardTitle>Modèles whisper.cpp (téléchargés à la demande)</CardTitle>
          </div>
          <CardDescription>
            Stockés dans %LOCALAPPDATA%/ClipForge/models/ — utilisés uniquement sur les fenêtres candidates (+15 s).
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2.5">
          {whisperModelsQuery.data?.map((m) => {
            const active = selectedModelQuery.data === m.id;
            return (
              <div
                key={m.id}
                className="flex flex-col gap-2 rounded-xl border border-border/60 bg-surface-subtle/35 p-3 sm:flex-row sm:items-center sm:justify-between"
              >
                <div className="space-y-0.5">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-fg">{m.label}</span>
                    {m.downloaded && (
                      <Badge variant="success">
                        <CheckCircle2 className="h-3 w-3" />
                        Téléchargé
                      </Badge>
                    )}
                    {active && <Badge variant="cyan">Actif</Badge>}
                  </div>
                  <p className="font-mono text-[11px] text-fg-subtle">{m.filename}</p>
                </div>
                <div className="flex items-center gap-2">
                  {!m.downloaded && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={downloadingModel !== null}
                      onClick={() => void handleDownloadWhisper(m.id)}
                    >
                      <Download className="h-3.5 w-3.5" />
                      {downloadingModel === m.id ? "Téléchargement…" : "Télécharger"}
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant={active ? "primary" : "secondary"}
                    onClick={() => void handleSelectWhisper(m.id)}
                  >
                    {active ? "Sélectionné" : "Choisir"}
                  </Button>
                </div>
              </div>
            );
          })}
        </CardContent>
      </Card>

      {/* Pondérations locales & emotes */}
      {weights && (
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Sliders className="h-4 w-4 text-accent-violet" />
              <CardTitle>Pondération des signaux locaux & emotes</CardTitle>
            </div>
            <CardDescription>
              Défaut : chat 35 %, audio 20 %, emotes 20 %, événements 10 %, voix 10 %, scène 5 %.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              {(
                [
                  ["chat", "Chat"],
                  ["audio", "Audio (LUFS)"],
                  ["emotes", "Emotes / rires"],
                  ["events", "Événements"],
                  ["voice", "Voix (VAD)"],
                  ["scene", "Scènes"],
                ] as const
              ).map(([field, label]) => (
                <div key={field} className="space-y-1">
                  <label className="text-xs text-fg-muted">
                    {label} ({Math.round(weights[field] * 100)} %)
                  </label>
                  <input
                    type="range"
                    min={0}
                    max={100}
                    value={Math.round(weights[field] * 100)}
                    onChange={(e) =>
                      setWeights({ ...weights, [field]: Number(e.target.value) / 100 })
                    }
                    className="w-full accent-violet-500"
                  />
                </div>
              ))}
            </div>

            <div className="space-y-1">
              <label className="text-xs text-fg-muted">
                Liste d'emotes suivies (séparées par des virgules)
              </label>
              <input
                type="text"
                value={weights.emotes_list.join(", ")}
                onChange={(e) =>
                  setWeights({
                    ...weights,
                    emotes_list: e.target.value
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
                className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 font-mono text-xs text-fg"
              />
            </div>

            <div className="flex justify-end">
              <Button size="sm" variant="primary" onClick={() => void handleSaveWeights()}>
                Enregistrer les signaux
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Limiteur de débit Gemini RPM/TPM */}
      {rateLimit && (
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Gauge className="h-4 w-4 text-accent-cyan" />
              <CardTitle>Limiteur de débit API Gemini (RPM / TPM)</CardTitle>
            </div>
            <CardDescription>
              Ajustez selon votre palier Google AI Studio pour éviter les erreurs HTTP 429.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
              <div className="space-y-1">
                <label className="text-xs text-fg-muted">Max requêtes / min (RPM)</label>
                <input
                  type="number"
                  min={1}
                  value={rateLimit.max_rpm}
                  onChange={(e) =>
                    setRateLimit({ ...rateLimit, max_rpm: Number(e.target.value) })
                  }
                  className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 font-mono text-xs text-fg"
                />
              </div>
              <div className="space-y-1">
                <label className="text-xs text-fg-muted">Max tokens / min (TPM)</label>
                <input
                  type="number"
                  min={10000}
                  step={10000}
                  value={rateLimit.max_tpm}
                  onChange={(e) =>
                    setRateLimit({ ...rateLimit, max_tpm: Number(e.target.value) })
                  }
                  className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 font-mono text-xs text-fg"
                />
              </div>
              <div className="space-y-1">
                <label className="text-xs text-fg-muted">Relances 429/503</label>
                <input
                  type="number"
                  min={1}
                  max={6}
                  value={rateLimit.max_retries}
                  onChange={(e) =>
                    setRateLimit({ ...rateLimit, max_retries: Number(e.target.value) })
                  }
                  className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 px-3 font-mono text-xs text-fg"
                />
              </div>
            </div>
            <div className="flex justify-end">
              <Button size="sm" variant="primary" onClick={() => void handleSaveRateLimit()}>
                Enregistrer les limites
              </Button>
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
