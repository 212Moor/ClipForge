import type { GeminiModelInfo } from "@/bindings";

// Seuls les identifiants stables (sans suffixe preview, image, live, tts…) sont recommandés.
const FLASH_RE = /^gemini-(\d+(?:\.\d+)*)-flash$/;
const FLASH_LITE_RE = /^gemini-(\d+(?:\.\d+)*)-flash-lite$/;

function parseVersion(id: string, re: RegExp): number[] | null {
  const match = re.exec(id);
  return match ? match[1].split(".").map(Number) : null;
}

function compareVersions(a: number[], b: number[]): number {
  const len = Math.max(a.length, b.length);
  for (let i = 0; i < len; i += 1) {
    const diff = (a[i] ?? 0) - (b[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

function pickLatest(models: GeminiModelInfo[], re: RegExp): string | null {
  let best: { id: string; version: number[] } | null = null;
  for (const model of models) {
    const version = parseVersion(model.id, re);
    if (version && (!best || compareVersions(version, best.version) > 0)) {
      best = { id: model.id, version };
    }
  }
  return best?.id ?? null;
}

/** Étage A : gamme Flash-Lite la plus récente. Étage B : gamme Flash la plus récente. */
export function recommendModels(models: GeminiModelInfo[]): {
  stageA: string | null;
  stageB: string | null;
} {
  return {
    stageA: pickLatest(models, FLASH_LITE_RE),
    stageB: pickLatest(models, FLASH_RE),
  };
}
