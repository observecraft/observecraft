# ObserveCraft Web

Datadog-style observability UI (own brand). React 18 + Vite + TypeScript + ECharts.

```bash
npm install
npm run dev      # http://localhost:5173
npm run build
```

## Layout

- `src/theme/tokens.css` — **single source of truth** for colors, fonts, spacing, sizes. Change the look here.
- `src/theme/chartTheme.ts` — ECharts theme built from the tokens (axis, grid, tooltip, palette).
- `src/components/shell/` — sidebar, top bar, page header + global time picker (`useTime()`).
- `src/components/charts/Chart.tsx` — themed, auto-resizing ECharts wrapper.
- `src/pages/logs/` — Logs product: `LogsLayout` (tabs), `LogExplorer`, `LiveTail`, shared `QueryBar`
  (tokenised search + autocomplete), `LogTable`, `LogMessage` (syntax highlighting), `LogDetail` (side panel), `FacetPanel`.
- `src/lib/logQuery.ts` — search syntax parser/matcher (`key:value`, `key:(a OR b)`, `key:abc*`, `-key:x`,
  `@attr.path:x`, free text). The query string is the single source of truth: facets edit it.
- `src/pages/dashboards/` — dashboard grid + widget types (timeseries, query value, top list, heatmap).
- `src/api/types.ts` — the wire contract the backend must implement.
- `src/api/client.ts` — data access; currently backed by `api/mock/` (generator + in-memory store + simulated
  live stream). Swap to `fetch()` / `EventSource` when the query API exists.

## Error Tracking (`/errors`) — reference pattern for new features

- `src/api/errorTracking/types.ts` — contract (cursor-paged search, facets, issue detail, samples, mutations).
- `src/api/errorTracking/client.ts` — one function per endpoint (paths documented at the top); backed by `api/mock/errorTracking.ts`.
- `src/pages/errors/queries.ts` — TanStack Query hooks. Components use hooks, never the client directly; mutations
  invalidate `['error-tracking']`.
- Shareable state (`q`, `state`, `sort`, `issue`) lives in the URL; transient UI state stays in components.
- Every data region renders loading / error / empty states (`components/ui/AsyncState.tsx`).

## Shared building blocks

- `components/query/QueryBar` — tokenised search with pluggable `suggest()`.
- `components/facets/FacetPanel` — `include` (Logs) or `exclude` (Error Tracking) checkbox semantics.
- `components/json/JsonTree` — collapsible attribute tree.
- `components/ui/Dropdown` — `Select` / `Menu` / `usePopover`.
- `lib/searchQuery.ts` — parse/format/edit queries only. Evaluation is backend-side (`api/mock/match.ts` in the mock).

## Integrations (`/integrations`)

- `src/api/integrations/{types,client}.ts` — catalog search, detail, install/uninstall, marketplace.
- An integration = collector config (OpenTelemetry Collector receiver and/or Vector source) + metadata + bundled
  dashboards/monitors. The Configure tab shows the exact fragment "Install" rolls out — this is the contract the
  fleet manager / collector backend must implement. Generated YAML in the mock is a template; verify receiver
  fields against the pinned collector version.
- Logos: `pages/integrations/IntegrationLogo.tsx` maps logo keys to Simple Icons (CC0 artwork; brands remain
  trademarks of their owners, used only to identify the integration) with wordmark/tile fallbacks.
- Shared: `components/ui/Modal`, `components/code/CodeBlock` (YAML highlighting + copy).

## Infrastructure (`/infrastructure`)

- `src/api/infrastructure/{types,client}.ts` — host search (tag query + metric range filters, group-by, sort,
  offset paging), host detail, host metrics, pods / containers / processes (paged, filterable).
- Host list state (`q`, `group`, `sort`, `offset`, `host`) lives in the URL.
- The host panel's sections are independent queries, so each loads/fails on its own.
- Hosts in the mock include the ones emitting mock logs, so the panel's Logs section and "Open Resource → Logs" have data.
- Shared: `components/ui/Pager` (pager, `formatBytes`, `formatAge`, `UsageBar`), `lib/time.ts` (`snapRange`).

## Dashboards (`/dashboards`, `/dashboards/:id`)

- **Document model** (`src/api/dashboards/types.ts`): a dashboard is JSON — `layoutType` (`grid` 12-col / `auto`
  timeboard / `free` screenboard), `templateVariables`, and a widget tree (`group` widgets contain widgets).
  This JSON is what the backend stores; Export/Import in the UI round-trips it.
- **Widgets**: timeseries, query value (conditional formats), top list, change, bar chart, table (multi-query
  columns), heatmap, distribution, pie, treemap, note (safe Markdown subset), image, group, monitor summary, log stream.
- **Metric query language** (`src/lib/metricQuery.ts`): `avg:metric{tag:value,$var} by {tag}`. Template variables
  are resolved in the browser, so the query API only ever receives plain queries.
- **Query API contract**: timeseries, scalar (time-aggregated), grouped scalar (top lists/tables/pies) and
  distribution endpoints — see `src/api/dashboards/client.ts`. The mock evaluator (`api/mock/metricsEngine.ts`)
  documents the expected semantics (filter → group by → space aggregation → time aggregation).
- Editing: drag/resize (react-grid-layout) and every change auto-saves (optimistic update); integration
  dashboards are read-only until cloned. Template variable values live in the URL (`?tpl_var_env=prod`).
- The mock persists dashboards to `localStorage` (`oc.mock.dashboards.vN`); bump the version when seed
  definitions change.

## Brand logos

- Full-colour logos come from **SVG Logos** (`@iconify-json/logos`, CC0). `scripts/build-logos.mjs` extracts only the
  logos we use into `src/assets/brandLogos.generated.ts` (≈215 KB raw, ≈70 KB gzip). To add one: add a key to `MAP`
  in the script and run `npm run logos`.
- Render with `components/brand/BrandLogo` (unique SVG ids per instance; `maxRatio` for wide wordmarks).
- Fallbacks: Simple Icons in brand colour (single-colour marks) → wordmark/tile in `IntegrationLogo`.
- Logos are trademarks of their owners and are shown only to identify the technology.
