import { useCallback, useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  CheckCircle2,
  ExternalLink,
  Eye,
  EyeOff,
  Info,
  KeyRound,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { commands, type GeminiModelInfo, type GeminiModelSelection } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { recommendModels } from "@/lib/gemini-models";

const SELECT_CLASS =
  "h-9 w-full rounded-xl border border-border bg-surface-elevated px-3 text-xs text-fg focus:border-accent-cyan focus:outline-none";

export function GeminiSettingsCard() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [apiKeyInput, setApiKeyInput] = useState("");
  const [revealInput, setRevealInput] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testBanner, setTestBanner] = useState<{ ok: boolean; text: string } | null>(null);
  const [models, setModels] = useState<GeminiModelInfo[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);

  const keyStatusQuery = useQuery({
    queryKey: ["gemini-key-status"],
    queryFn: async () => {
      const res = await commands.getGeminiKeyStatus();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const selectionQuery = useQuery({
    queryKey: ["gemini-model-selection"],
    queryFn: async () => {
      const res = await commands.getGeminiModelSelection();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const fetchModels = useCallback(async () => {
    setLoadingModels(true);
    try {
      const res = await commands.listGeminiModels();
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      setModels(res.data);
    } finally {
      setLoadingModels(false);
    }
  }, []);

  const configured = keyStatusQuery.data?.configured ?? false;

  // Charge la liste réelle des modèles quand une clé est déjà enregistrée (0 token facturé).
  useEffect(() => {
    if (configured) void fetchModels();
  }, [configured, fetchModels]);

  const recommended = useMemo(() => recommendModels(models), [models]);
  const sortedModels = useMemo(() => [...models].sort((a, b) => a.id.localeCompare(b.id)), [models]);

  const handleSaveKey = async () => {
    const trimmed = apiKeyInput.trim();
    if (!trimmed) return;
    setSaving(true);
    try {
      const res = await commands.saveGeminiApiKey(trimmed);
      if (res.status === "error") {
        toast.error(res.error.message);
        return;
      }
      setApiKeyInput("");
      toast.success(t("gemini.savedKeyToast"));
      await queryClient.invalidateQueries({ queryKey: ["gemini-key-status"] });
      await fetchModels();
    } finally {
      setSaving(false);
    }
  };

  const handleTestKey = async () => {
    setTesting(true);
    setTestBanner(null);
    try {
      const override = apiKeyInput.trim().length > 0 ? apiKeyInput.trim() : null;
      const res = await commands.testGeminiApiKey(override);
      if (res.status === "error") {
        setTestBanner({ ok: false, text: res.error.message });
        return;
      }
      setTestBanner({ ok: true, text: res.data.message });
      setModels(res.data.models);
    } finally {
      setTesting(false);
    }
  };

  const handleDeleteKey = async () => {
    const res = await commands.deleteGeminiApiKey();
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    setModels([]);
    setTestBanner(null);
    toast.success(t("gemini.deletedKeyToast"));
    await queryClient.invalidateQueries({ queryKey: ["gemini-key-status"] });
  };

  const saveSelection = async (next: GeminiModelSelection) => {
    const res = await commands.setGeminiModelSelection(next);
    if (res.status === "error") {
      toast.error(res.error.message);
      return;
    }
    toast.success(t("gemini.modelsSavedToast"));
    await queryClient.invalidateQueries({ queryKey: ["gemini-model-selection"] });
  };

  const selection: GeminiModelSelection = selectionQuery.data ?? {
    stage_a_model: null,
    stage_b_model: null,
  };

  const handleModelChange = (field: keyof GeminiModelSelection, value: string) =>
    void saveSelection({ ...selection, [field]: value || null });

  const canApplyRecommended =
    (recommended.stageA !== null || recommended.stageB !== null) &&
    (recommended.stageA !== selection.stage_a_model ||
      recommended.stageB !== selection.stage_b_model);

  const status = keyStatusQuery.data;

  const renderOptions = (recommendedId: string | null) =>
    sortedModels.map((m) => (
      <option key={m.name} value={m.id}>
        {m.display_name} ({m.id}) · {Math.round(m.input_token_limit / 1000)}k
        {m.id === recommendedId ? ` · ${t("gemini.recommendedSuffix")}` : ""}
      </option>
    ));

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <KeyRound className="h-4 w-4 text-accent-cyan" aria-hidden="true" />
            <CardTitle>{t("gemini.title")}</CardTitle>
          </div>
          {status && (
            <Badge variant={status.configured ? "success" : "warning"}>
              <ShieldCheck className="h-3 w-3" />
              {status.configured
                ? `${t("gemini.statusConfigured")} (${status.masked_preview ?? ""})`
                : t("gemini.statusMissing")}
            </Badge>
          )}
        </div>
        <CardDescription>{t("gemini.subtitle")}</CardDescription>
      </CardHeader>

      <CardContent className="space-y-5">
        <div className="flex flex-col gap-2 rounded-xl border border-accent-violet/35 bg-accent-violet/10 p-3.5 text-xs leading-relaxed text-fg sm:flex-row sm:items-start sm:justify-between">
          <div className="flex items-start gap-2.5">
            <Info className="mt-0.5 h-4 w-4 shrink-0 text-accent-violet" aria-hidden="true" />
            <div className="space-y-0.5">
              <p className="font-semibold">{t("gemini.proNoticeTitle")}</p>
              <p className="text-fg-muted">{t("gemini.proNoticeBody")}</p>
            </div>
          </div>
          <Button
            size="sm"
            variant="outline"
            className="shrink-0"
            onClick={() => void openUrl("https://aistudio.google.com/apikey")}
          >
            <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
            {t("gemini.openAiStudio")}
          </Button>
        </div>

        <div className="space-y-2">
          <label htmlFor="gemini-api-key" className="text-sm font-medium text-fg">
            {t("gemini.inputLabel")}
          </label>
          <div className="flex flex-col gap-2 sm:flex-row">
            <div className="relative flex-1">
              <input
                id="gemini-api-key"
                type={revealInput ? "text" : "password"}
                value={apiKeyInput}
                onChange={(e) => setApiKeyInput(e.target.value)}
                placeholder={t("gemini.inputPlaceholder")}
                autoComplete="off"
                spellCheck={false}
                className="h-9 w-full rounded-xl border border-border bg-surface-subtle/60 pl-3 pr-9 font-mono text-xs text-fg placeholder:text-fg-subtle focus:border-accent-cyan focus:outline-none"
              />
              <button
                type="button"
                onClick={() => setRevealInput((v) => !v)}
                aria-label={revealInput ? t("gemini.hide") : t("gemini.show")}
                className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md p-1 text-fg-subtle hover:text-fg"
              >
                {revealInput ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
              </button>
            </div>

            <div className="flex flex-wrap gap-2">
              <Button
                variant="primary"
                disabled={saving || apiKeyInput.trim().length === 0}
                onClick={() => void handleSaveKey()}
              >
                {t("gemini.saveKey")}
              </Button>
              <Button
                variant="secondary"
                disabled={testing || (apiKeyInput.trim().length === 0 && !configured)}
                onClick={() => void handleTestKey()}
              >
                <CheckCircle2 className="h-3.5 w-3.5" aria-hidden="true" />
                {t("gemini.testKey")}
              </Button>
              {configured && (
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("gemini.deleteKey")}
                  title={t("gemini.deleteKey")}
                  onClick={() => void handleDeleteKey()}
                >
                  <Trash2 className="h-4 w-4 text-status-danger" />
                </Button>
              )}
            </div>
          </div>
        </div>

        {testBanner && (
          <div
            role="status"
            className={`rounded-xl border px-3.5 py-2.5 text-xs ${
              testBanner.ok
                ? "border-status-success/40 bg-status-success/10 text-fg"
                : "border-status-danger/40 bg-status-danger/10 text-fg"
            }`}
          >
            {testBanner.text}
          </div>
        )}

        <div className="space-y-3 border-t border-border/60 pt-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 className="text-sm font-medium text-fg">{t("gemini.modelsTitle")}</h3>
            <div className="flex flex-wrap gap-2">
              {canApplyRecommended && (
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() =>
                    void saveSelection({
                      stage_a_model: recommended.stageA,
                      stage_b_model: recommended.stageB,
                    })
                  }
                >
                  {t("gemini.applyRecommended")}
                </Button>
              )}
              <Button
                size="sm"
                variant="outline"
                disabled={!configured || loadingModels}
                onClick={() => void fetchModels()}
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${loadingModels ? "animate-spin" : ""}`}
                  aria-hidden="true"
                />
                {t("gemini.refreshModels")}
              </Button>
            </div>
          </div>

          {models.length === 0 ? (
            <p className="text-xs text-fg-muted">{t("gemini.modelsEmpty")}</p>
          ) : (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <label htmlFor="stage-a-model" className="text-xs font-medium text-fg-muted">
                  {t("gemini.stageALabel")}
                </label>
                <select
                  id="stage-a-model"
                  value={selection.stage_a_model ?? ""}
                  onChange={(e) => handleModelChange("stage_a_model", e.target.value)}
                  className={SELECT_CLASS}
                >
                  <option value="">{t("gemini.selectPlaceholder")}</option>
                  {renderOptions(recommended.stageA)}
                </select>
              </div>

              <div className="space-y-1.5">
                <label htmlFor="stage-b-model" className="text-xs font-medium text-fg-muted">
                  {t("gemini.stageBLabel")}
                </label>
                <select
                  id="stage-b-model"
                  value={selection.stage_b_model ?? ""}
                  onChange={(e) => handleModelChange("stage_b_model", e.target.value)}
                  className={SELECT_CLASS}
                >
                  <option value="">{t("gemini.selectPlaceholder")}</option>
                  {renderOptions(recommended.stageB)}
                </select>
              </div>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
