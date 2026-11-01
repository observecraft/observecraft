import { useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Bell, ChevronsUpDown, ExternalLink, Eye, Globe, Search } from 'lucide-react';
import type { SyntheticEndpoint, SyntheticTest } from '../../api/synthetics/client';
import { TYPE_LABEL } from '../../api/synthetics/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact, timeAgo } from '../../lib/format';
import { TestStatus, TypeIcon, fmtMs } from './common';
import { useSyntheticCoverage, useSyntheticTests } from './queries';
import { SyntheticsIntro } from './SyntheticsIntro';

const minute = 60_000;
const WEEK = 7 * 86_400_000;
const LOCATION_NAME: Record<string, string> = { 'observecraft-server': 'ObserveCraft server' };

type GroupBy = 'location' | 'type' | 'env' | 'team' | 'service';
const GROUP_LABEL: Record<GroupBy, string> = { location: 'Location', type: 'Type', env: 'Env', team: 'Team', service: 'Service' };

/** The values a test is grouped under (a test appears under every location it runs from). */
function groupsOf(t: SyntheticTest, by: GroupBy): string[] {
  switch (by) {
    case 'location':
      return t.locations.map((l) => LOCATION_NAME[l] ?? l);
    case 'type':
      return [TYPE_LABEL[t.type]];
    case 'env':
      return [t.env || 'no env'];
    case 'team':
      return [t.team || 'no team'];
    case 'service': {
      const s = t.tags.filter((x) => x.startsWith('service:')).map((x) => x.slice(8));
      return s.length ? s : ['no service'];
    }
  }
}

/** Synthetic Monitoring & Testing → Overview: the health of the org's tests and the endpoints that lack one. */
export function SyntheticsOverviewPage() {
  const [team, setTeam] = useState('');
  const [env, setEnv] = useState('');
  // The state is "now": the uptime of the last hour comes along for the treemap's tooltips.
  const [range] = useState(() => {
    const to = Math.floor(Date.now() / minute) * minute;
    return { from: to - 60 * minute, to };
  });
  const [week] = useState(() => {
    const to = Math.floor(Date.now() / minute) * minute;
    return { from: to - WEEK, to };
  });
  const tests = useSyntheticTests({ query: '', range });
  const coverage = useSyntheticCoverage({ range: week, env: env || null });

  if (tests.isPending) return <Loading label="Loading synthetic tests…" />;
  if (tests.isError) return <ErrorState error={tests.error} onRetry={() => tests.refetch()} />;
  const all = tests.data.tests;
  const teams = [...new Set(all.map((t) => t.team).filter(Boolean))].sort();
  const envs = [...new Set(all.map((t) => t.env).filter(Boolean))].sort();
  const scoped = all.filter((t) => (!team || t.team === team) && (!env || t.env === env));
  const gaps = <CoverageGaps endpoints={coverage.data?.endpoints} error={coverage.error as Error | null} loading={coverage.isPending} />;
  if (all.length === 0) return <SyntheticsIntro>{gaps}</SyntheticsIntro>;

  return (
    <div className="syn-page syn-overview">
      <div className="syn-ov-filters">
        <label className="syn-select-field">
          <span>Teams</span>
          <select value={team} onChange={(e) => setTeam(e.target.value)}>
            <option value="">All</option>
            {teams.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        </label>
        <label className="syn-select-field">
          <span>Environment</span>
          <select value={env} onChange={(e) => setEnv(e.target.value)}>
            <option value="">All</option>
            {envs.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        </label>
      </div>
      <TestHealth tests={scoped} />
      {gaps}
    </div>
  );
}

// ================================================================ test health

function TestHealth({ tests }: { tests: SyntheticTest[] }) {
  const [tag, setTag] = useState('');
  const [creator, setCreator] = useState('');
  const [by, setBy] = useState<GroupBy>('location');
  const running = tests.filter((t) => !t.paused);
  const tags = [...new Set(running.flatMap((t) => t.tags))].sort();
  const creators = [...new Set(running.map((t) => t.createdBy ?? '').filter(Boolean))].sort();
  const shown = running.filter((t) => (!tag || t.tags.includes(tag)) && (!creator || t.createdBy === creator));
  const alerting = shown.filter((t) => t.status === 'alert');
  const groups = (pred: (t: SyntheticTest) => boolean) => {
    const m = new Map<string, SyntheticTest[]>();
    for (const t of shown.filter(pred)) for (const g of groupsOf(t, by)) m.set(g, [...(m.get(g) ?? []), t]);
    return [...m.entries()].map(([name, ts]) => ({ name, tests: ts })).sort((a, b) => b.tests.length - a.tests.length || a.name.localeCompare(b.name));
  };
  const ok = groups((t) => t.status !== 'alert');
  const alert = groups((t) => t.status === 'alert');
  const tiles = ok.length + alert.length;
  const okWeight = ok.reduce((n, g) => n + g.tests.length, 0);
  const alertWeight = alert.reduce((n, g) => n + g.tests.length, 0);
  const covered = new Set([...ok, ...alert].flatMap((g) => g.tests.map((t) => t.id))).size;

  return (
    <section className="syn-ov-section">
      <header className="syn-ov-head">
        <span className="syn-ov-icon">
          <Bell size={18} />
        </span>
        <div>
          <h3>Test health</h3>
          <p className="muted">
            {alerting.length} of {running.length} test{running.length === 1 ? ' is' : 's are'} in alert right now.{' '}
            <Link to="/synthetics/tests?q=status:alert">
              View all alerting tests <ExternalLink size={13} />
            </Link>
          </p>
        </div>
      </header>
      <div className="syn-ov-card syn-health">
        <div className="syn-health-main">
          <div className="syn-ov-filters">
            <label className="syn-select-field">
              <span>Tags</span>
              <select value={tag} onChange={(e) => setTag(e.target.value)}>
                <option value="">All</option>
                {tags.map((t) => (
                  <option key={t}>{t}</option>
                ))}
              </select>
            </label>
            <label className="syn-select-field">
              <span>Creator</span>
              <select value={creator} onChange={(e) => setCreator(e.target.value)}>
                <option value="">All</option>
                {creators.map((t) => (
                  <option key={t}>{t}</option>
                ))}
              </select>
            </label>
            <span className="syn-meta-sep" />
            <label className="syn-select-field">
              <span>Group by</span>
              <select value={by} onChange={(e) => setBy(e.target.value as GroupBy)}>
                {(Object.keys(GROUP_LABEL) as GroupBy[]).map((g) => (
                  <option key={g} value={g}>
                    {GROUP_LABEL[g]}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <p className="muted syn-health-note">
            Each tile is a {GROUP_LABEL[by].toLowerCase()} sized by its running tests; a test counts under every{' '}
            {by === 'location' ? 'location it runs from' : `${GROUP_LABEL[by].toLowerCase()} it has`}, and under In alert while it fails. ({tiles} tile{tiles === 1 ? '' : 's'} from {covered} test
            {covered === 1 ? '' : 's'}, {alerting.length} of them failing.)
          </p>
          <div className="syn-treemap-head">
            <b>Total</b>
            <b>
              {shown.length} test{shown.length === 1 ? '' : 's'}
            </b>
          </div>
          {shown.length === 0 ? (
            <p className="mon-empty">No running test matches these filters.</p>
          ) : (
            <div className="syn-treemap">
              {ok.length > 0 && <Region label="OK" status="ok" groups={ok} weight={okWeight} share={okWeight / (okWeight + alertWeight)} />}
              {alert.length > 0 && <Region label="In alert" status="alert" groups={alert} weight={alertWeight} share={alertWeight / (okWeight + alertWeight)} />}
            </div>
          )}
        </div>
        <aside className="syn-health-side">
          <h4>Tests in alert</h4>
          {alerting.length === 0 ? (
            <p className="muted">No tests in alert.</p>
          ) : (
            alerting.map((t) => (
              <Link key={t.id} to={`/synthetics/tests/${t.id}`} className="syn-alert-row">
                <TestStatus status={t.status} paused={t.paused} small />
                <span className="syn-alert-name">
                  <span>{t.name}</span>
                  <span className="muted">
                    <TypeIcon type={t.type} size={13} /> {TYPE_LABEL[t.type]} · {t.domain}
                    {t.failingSince ? ` · failing since ${timeAgo(t.failingSince)}` : ''}
                  </span>
                </span>
              </Link>
            ))
          )}
        </aside>
      </div>
    </section>
  );
}

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Splits `rect` among `weights` (largest first), alternating cuts along the longer side. */
function slice(weights: number[], rect: Rect): Rect[] {
  if (weights.length === 0) return [];
  if (weights.length === 1) return [rect];
  const total = weights.reduce((a, b) => a + b, 0);
  let acc = 0;
  let cut = 1;
  for (; cut < weights.length; cut++) {
    if (acc + weights[cut - 1] >= total / 2) break;
    acc += weights[cut - 1];
  }
  acc = weights.slice(0, cut).reduce((a, b) => a + b, 0);
  const share = acc / total;
  const [a, b] =
    rect.w >= rect.h
      ? [
          { ...rect, w: rect.w * share },
          { ...rect, x: rect.x + rect.w * share, w: rect.w * (1 - share) },
        ]
      : [
          { ...rect, h: rect.h * share },
          { ...rect, y: rect.y + rect.h * share, h: rect.h * (1 - share) },
        ];
  return [...slice(weights.slice(0, cut), a), ...slice(weights.slice(cut), b)];
}

function Region({ label, status, groups, weight, share }: { label: string; status: 'ok' | 'alert'; groups: { name: string; tests: SyntheticTest[] }[]; weight: number; share: number }) {
  const navigate = useNavigate();
  // Laid out on the region's nominal size (its share of a 1000×420 box), then drawn in percentages of it.
  const W = Math.max(14, 1000 * share);
  const rects = slice(
    groups.map((g) => g.tests.length),
    { x: 0, y: 0, w: W, h: 420 },
  );
  return (
    <div className={`syn-region ${status}`} style={{ flexGrow: weight }}>
      <div className="syn-region-head">
        <span>{label}</span>
        <span>
          {weight} test{weight === 1 ? '' : 's'}
        </span>
      </div>
      <div className="syn-region-body">
        {groups.map((g, i) => {
          const r = rects[i];
          const big = r.w > 260 && r.h > 160;
          const small = r.w < 70 || r.h < 40;
          return (
            <button
              key={g.name}
              className="syn-region-tile"
              style={{ left: `${(r.x / W) * 100}%`, top: `${(r.y / 420) * 100}%`, width: `${(r.w / W) * 100}%`, height: `${(r.h / 420) * 100}%` }}
              title={`${g.name}: ${g.tests.map((t) => t.name).join(', ')}`}
              onClick={() => navigate(`/synthetics/tests?q=${encodeURIComponent(`status:${status === 'ok' ? 'ok' : 'alert'}`)}`)}
            >
              <span className={big ? 'big' : small ? 'small' : ''}>
                <span className="syn-region-name">{g.name}</span>
                <b>
                  {g.tests.length} test{g.tests.length === 1 ? '' : 's'}
                </b>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

// ================================================================ coverage gaps

const ENDPOINT_GRID = 400;

function CoverageGaps({ endpoints, error, loading }: { endpoints?: SyntheticEndpoint[]; error: Error | null; loading: boolean }) {
  const [search, setSearch] = useState('');
  const [grouped, setGrouped] = useState(true);
  const all = endpoints ?? [];
  // High traffic: the top tenth of endpoints by requests.
  const threshold = useMemo(() => {
    const sorted = all.map((e) => e.requests).sort((a, b) => b - a);
    return sorted[Math.max(0, Math.ceil(sorted.length / 10) - 1)] ?? Infinity;
  }, [all]);
  const services = useMemo(() => {
    const m = new Map<string, { requests: number; endpoints: SyntheticEndpoint[] }>();
    for (const e of all) {
      const s = m.get(e.service) ?? { requests: 0, endpoints: [] };
      s.requests += e.requests;
      s.endpoints.push(e);
      m.set(e.service, s);
    }
    return [...m.entries()]
      .sort((a, b) => b[1].requests - a[1].requests)
      .slice(0, 20)
      .map(([name, s]) => ({ name, ...s }));
  }, [all]);
  const inTop = new Set(services.map((s) => s.name));
  const matches = (e: SyntheticEndpoint) => `${e.service} ${e.method} ${e.route}`.toLowerCase().includes(search.trim().toLowerCase());
  const scoped = all.filter((e) => inTop.has(e.service));
  const untested = scoped.filter((e) => e.tests.length === 0);
  const high = untested.filter((e) => e.requests >= threshold).length;
  const ranked = untested.filter(matches).slice(0, 50);
  const kind = (e: SyntheticEndpoint) => (e.tests.length ? 'covered' : e.requests >= threshold ? 'high' : 'low');

  return (
    <section className="syn-ov-section">
      <header className="syn-ov-head">
        <span className="syn-ov-icon">
          <Eye size={18} />
        </span>
        <div className="syn-grow">
          <h3>Coverage gaps</h3>
          <p className="muted">Your top 20 services by traffic over the past 1 week, and where they lack tests.</p>
        </div>
        <span className="syn-ov-search">
          <Search size={15} />
          <input placeholder="Search services or endpoints" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search services or endpoints" />
        </span>
        <span className="syn-seg">
          <button className="active">Endpoints</button>
          <button disabled title="View coverage needs Real User Monitoring, which isn't available yet">
            Views
          </button>
        </span>
      </header>
      <div className="syn-ov-card syn-gaps">
        {loading ? (
          <Loading label="Loading endpoints…" />
        ) : error ? (
          <p className="form-error">{error.message}</p>
        ) : scoped.length === 0 ? (
          <p className="mon-empty">No HTTP endpoint served traffic in APM over the past week. Endpoints appear here once services send traces.</p>
        ) : (
          <>
            <div className="syn-gaps-head">
              <div>
                <b>
                  {untested.length} untested endpoint{untested.length === 1 ? '' : 's'}
                </b>{' '}
                <span className="muted">out of {scoped.length}</span>
                <div className="syn-gaps-legend">
                  <span>
                    <i className="syn-sq high" /> {compact(high)} high traffic, no test
                  </span>
                  <span>
                    <i className="syn-sq low" /> {compact(untested.length - high)} lower traffic, no test
                  </span>
                  <span>
                    <i className="syn-sq covered" /> {compact(scoped.length - untested.length)} covered by a test
                  </span>
                </div>
              </div>
              <label className="syn-switch">
                <input type="checkbox" checked={grouped} onChange={() => setGrouped((g) => !g)} />
                <span /> Group by service
              </label>
            </div>
            <div className="syn-gaps-body">
              <div className="syn-gaps-list">
                <div className="syn-gaps-list-head muted">Untested endpoints ranked by traffic, past 1 week</div>
                {ranked.length === 0 ? (
                  <p className="muted syn-gaps-none">{untested.length ? 'No untested endpoint matches the search.' : 'Every endpoint of these services has a test.'}</p>
                ) : (
                  ranked.map((e) => (
                    <Link
                      key={`${e.service} ${e.method} ${e.route}`}
                      to={`/synthetics/create?url=${encodeURIComponent(e.route)}&method=${e.method || 'GET'}&service=${encodeURIComponent(e.service)}`}
                      className="syn-gap-row"
                      title="Create a test for this endpoint"
                    >
                      <span className="syn-gap-route">
                        {e.method && <span className="syn-method-chip">{e.method}</span>}
                        <span className="mono">{e.route}</span>
                      </span>
                      <span className="syn-icon-text">
                        <Globe size={14} className="muted" /> {e.service}
                      </span>
                      <span className="muted">
                        <b className="syn-strong">{compact(e.requests)}</b> requests · <b className="syn-strong">{compact(e.errors)}</b> errors · <b className="syn-strong">{fmtMs(e.p95Ms)}</b> p95
                      </span>
                    </Link>
                  ))
                )}
              </div>
              <div className={`syn-gaps-grid${grouped ? '' : ' flat'}`}>
                {grouped ? (
                  services
                    .filter((s) => !search || s.endpoints.some(matches))
                    .map((s) => {
                      const missing = s.endpoints.filter((e) => !e.tests.length).length;
                      return (
                        <div key={s.name} className="syn-svc-card">
                          <div className="syn-svc-head">
                            <Globe size={14} className="muted" />
                            <b title={s.name}>{s.name}</b>
                            <span className="muted">
                              {Math.round((missing / s.endpoints.length) * 100)}% of {compact(s.endpoints.length)} untested
                            </span>
                          </div>
                          <Squares endpoints={s.endpoints} kind={kind} matches={matches} />
                        </div>
                      );
                    })
                ) : (
                  <div className="syn-svc-card">
                    <Squares endpoints={scoped} kind={kind} matches={matches} />
                  </div>
                )}
              </div>
            </div>
          </>
        )}
      </div>
    </section>
  );
}

/** One square per endpoint, the most requested first; the rest folded after the first rows. */
function Squares({ endpoints, kind, matches }: { endpoints: SyntheticEndpoint[]; kind: (e: SyntheticEndpoint) => string; matches: (e: SyntheticEndpoint) => boolean }) {
  const [open, setOpen] = useState(false);
  const limit = open ? ENDPOINT_GRID * 5 : ENDPOINT_GRID;
  const sorted = [...endpoints].sort((a, b) => b.requests - a.requests);
  return (
    <>
      <div className="syn-squares">
        {sorted.slice(0, limit).map((e) => (
          <i
            key={`${e.method} ${e.route}`}
            className={`syn-sq ${kind(e)}${matches(e) ? '' : ' dim'}`}
            title={`${e.method} ${e.route}\n${compact(e.requests)} requests · ${compact(e.errors)} errors${e.tests.length ? `\nTested by ${e.tests.map((t) => t.name).join(', ')}` : ''}`}
          />
        ))}
      </div>
      {sorted.length > limit && (
        <button className="syn-divider-btn" onClick={() => setOpen(true)}>
          <ChevronsUpDown size={14} /> +{compact(sorted.length - limit)} more endpoints
        </button>
      )}
    </>
  );
}
