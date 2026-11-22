import { useQuery } from "@tanstack/react-query";
import { BarChart3, Coins, Database, Layers, RefreshCw } from "lucide-react";
import { commands } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { formatCurrency, formatTokens } from "@/lib/format";

export function CostsPage() {
  const costsQuery = useQuery({
    queryKey: ["costs-dashboard"],
    queryFn: async () => {
      const res = await commands.getCostsDashboardCmd();
      if (res.status === "error") throw new Error(res.error.message);
      return res.data;
    },
  });

  const data = costsQuery.data;
  const fx = data?.usd_to_eur ?? 0.92;
  const cacheRate =
    data && data.gauge.total_calls > 0
      ? Math.round((data.gauge.total_cache_hits / data.gauge.total_calls) * 100)
      : 0;

  return (
    <div className="space-y-6">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-fg">Coûts et tokens</h1>
          <p className="text-sm text-fg-muted">
            Chaque appel Gemini est mesuré, plafonné, mis en cache dans SQLite et valorisé via pricing.json.
          </p>
        </div>
        <Button
          size="sm"
          variant="outline"
          disabled={costsQuery.isFetching}
          onClick={() => void costsQuery.refetch()}
        >
          <RefreshCw className={`h-3.5 w-3.5 ${costsQuery.isFetching ? "animate-spin" : ""}`} />
          Actualiser
        </Button>
      </header>

      {/* KPIs */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Card className="p-4">
          <div className="flex items-center justify-between text-xs text-fg-subtle">
            <span>Aujourd'hui</span>
            <Coins className="h-3.5 w-3.5 text-accent-cyan" />
          </div>
          {costsQuery.isLoading ? (
            <Skeleton className="mt-2 h-7 w-28" />
          ) : (
            <div className="mt-1.5">
              <p className="font-mono text-xl font-semibold text-fg">
                {formatCurrency((data?.gauge.today_cost_usd ?? 0) * fx, "EUR")}
              </p>
              <p className="text-xs text-fg-muted">
                {formatTokens(data?.gauge.today_input_tokens ?? 0)} in ·{" "}
                {formatTokens(data?.gauge.today_output_tokens ?? 0)} out
              </p>
            </div>
          )}
        </Card>

        <Card className="p-4">
          <div className="flex items-center justify-between text-xs text-fg-subtle">
            <span>Ce mois-ci</span>
            <BarChart3 className="h-3.5 w-3.5 text-accent-violet" />
          </div>
          {costsQuery.isLoading ? (
            <Skeleton className="mt-2 h-7 w-28" />
          ) : (
            <div className="mt-1.5">
              <p className="font-mono text-xl font-semibold text-fg">
                {formatCurrency((data?.gauge.month_cost_usd ?? 0) * fx, "EUR")}
              </p>
              <p className="text-xs text-fg-muted">
                {formatTokens(data?.gauge.month_input_tokens ?? 0)} in ·{" "}
                {formatTokens(data?.gauge.month_output_tokens ?? 0)} out
              </p>
            </div>
          )}
        </Card>

        <Card className="p-4">
          <div className="flex items-center justify-between text-xs text-fg-subtle">
            <span>Cache SQLite SHA-256</span>
            <Database className="h-3.5 w-3.5 text-status-success" />
          </div>
          {costsQuery.isLoading ? (
            <Skeleton className="mt-2 h-7 w-24" />
          ) : (
            <div className="mt-1.5">
              <p className="font-mono text-xl font-semibold text-status-success">{cacheRate} %</p>
              <p className="text-xs text-fg-muted">
                {data?.gauge.total_cache_hits ?? 0} appels gratuits sur {data?.gauge.total_calls ?? 0}
              </p>
            </div>
          )}
        </Card>
      </div>

      {/* Par étage & Par projet */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <div className="flex items-center gap-2">
              <Layers className="h-4 w-4 text-accent-violet" />
              <CardTitle>Répartition par étage</CardTitle>
            </div>
            <CardDescription>Cascade Étage A / Étage B / régénérations ciblées.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {data && data.by_stage.length === 0 && (
              <p className="text-xs text-fg-muted">Aucun appel Gemini enregistré.</p>
            )}
            {data?.by_stage.map((st) => (
              <div
                key={st.stage}
                className="rounded-xl border border-border/60 bg-surface-subtle/35 p-3 text-xs space-y-1"
              >
                <div className="flex items-center justify-between">
                  <span className="font-mono font-semibold uppercase text-fg">{st.stage}</span>
                  <span className="font-mono text-accent-cyan">
                    {formatCurrency(st.cost_usd * fx, "EUR")}
                  </span>
                </div>
                <div className="flex flex-wrap gap-3 text-fg-muted">
                  <span>{st.calls} appels</span>
                  <span>{st.cache_hits} hits cache</span>
                  <span>
                    {formatTokens(st.input_tokens)} in / {formatTokens(st.output_tokens)} out
                  </span>
                  {st.cached_tokens > 0 && (
                    <span className="text-status-success">
                      {formatTokens(st.cached_tokens)} prefix cache
                    </span>
                  )}
                </div>
              </div>
            ))}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Budget par projet</CardTitle>
            <CardDescription>Consommation réelle vs plafond dur configuré.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {data && data.by_project.length === 0 && (
              <p className="text-xs text-fg-muted">Aucun projet créé.</p>
            )}
            {data?.by_project.map((p) => {
              const used = p.input_tokens + p.output_tokens;
              const pct = Math.min(100, Math.round((used / Math.max(1, p.token_budget_limit)) * 100));
              return (
                <div
                  key={p.project_id}
                  className="rounded-xl border border-border/60 bg-surface-subtle/35 p-3 text-xs space-y-1.5"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate font-semibold text-fg">{p.project_name}</span>
                    <Badge variant="default" className="uppercase text-[10px]">
                      {p.mode}
                    </Badge>
                  </div>
                  <div className="flex items-center justify-between text-fg-muted font-mono">
                    <span>
                      {formatTokens(used)} / {formatTokens(p.token_budget_limit)} tok ({pct}%)
                    </span>
                    <span className="text-accent-cyan">
                      {formatCurrency(p.cost_usd * fx, "EUR")}
                    </span>
                  </div>
                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-base">
                    <div
                      className="h-full rounded-full bg-gradient-to-r from-accent-violet to-accent-cyan"
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                </div>
              );
            })}
          </CardContent>
        </Card>
      </div>

      {/* Historique des appels */}
      <Card>
        <CardHeader>
          <CardTitle>Journal des appels Gemini ({data?.recent_calls.length ?? 0})</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead className="border-b border-border/70 text-fg-subtle">
                <tr>
                  <th className="py-2 pr-3">Étage</th>
                  <th className="py-2 pr-3">Modèle</th>
                  <th className="py-2 pr-3">Tokens (in/out)</th>
                  <th className="py-2 pr-3">Cache</th>
                  <th className="py-2 pr-3">Durée</th>
                  <th className="py-2 text-right">Coût (€)</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border/40 font-mono">
                {data?.recent_calls.map((c) => (
                  <tr key={c.id} className="text-fg-muted">
                    <td className="py-2 pr-3 uppercase text-fg">{c.stage}</td>
                    <td className="py-2 pr-3">{c.model}</td>
                    <td className="py-2 pr-3">
                      {formatTokens(c.input_tokens)} / {formatTokens(c.output_tokens)}
                    </td>
                    <td className="py-2 pr-3">
                      {c.cache_hit ? (
                        <Badge variant="success">HIT (0 tok)</Badge>
                      ) : c.cached_tokens > 0 ? (
                        <Badge variant="cyan">{formatTokens(c.cached_tokens)} prefix</Badge>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td className="py-2 pr-3">{c.duration_ms} ms</td>
                    <td className="py-2 text-right text-fg">
                      {formatCurrency(c.estimated_cost_usd * fx, "EUR")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
