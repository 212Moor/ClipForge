export function formatDuration(seconds: number): string {
  const safe = Math.max(0, Math.floor(seconds));
  const h = Math.floor(safe / 3600);
  const m = Math.floor((safe % 3600) / 60);
  const s = safe % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function formatCurrency(amount: number, currency: "EUR" | "USD" = "EUR"): string {
  const symbol = currency === "EUR" ? "€" : "$";
  if (amount > 0 && amount < 0.01) {
    return `< 0.01 ${symbol}`;
  }
  return `${amount.toFixed(3)} ${symbol}`;
}
