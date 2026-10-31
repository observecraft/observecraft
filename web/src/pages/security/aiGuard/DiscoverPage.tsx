import { useEffect, useMemo, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import { Cog, Globe, Maximize2, Minus, PanelRight, Plus, Search, ShieldAlert, ShieldCheck, ShieldOff } from 'lucide-react';
import type { AiProvider, AiService } from '../../../api/aiGuard/client';
import { BrandLogo, hasBrandLogo } from '../../../components/brand/BrandLogo';
import { TimePicker } from '../../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { useAiDiscover } from './queries';

const MIN = 60_000;

/** Provider keys (as the server normalizes them) → name; the grid lists them all, greyed when unused. */
export const PROVIDERS: [string, string][] = [
  ['openai', 'OpenAI'],
  ['anthropic', 'Anthropic'],
  ['google', 'Google'],
  ['deepseek', 'DeepSeek'],
  ['azureopenai', 'Azure OpenAI'],
  ['bedrock', 'Amazon Bedrock'],
  ['vertexai', 'Google Vertex AI'],
  ['mistral', 'Mistral AI'],
  ['cohere', 'Cohere'],
  ['meta', 'Meta'],
  ['huggingface', 'Hugging Face'],
  ['perplexity', 'Perplexity'],
];
const providerName = (k: string) => PROVIDERS.find(([p]) => p === k)?.[1] ?? k;
const logoKey = (k: string) => k;

export function ProviderLogo({ provider, size = 22 }: { provider: string; size?: number }) {
  const key = logoKey(provider);
  return hasBrandLogo(key) ? <BrandLogo name={key} size={size} title={providerName(provider)} /> : <Cog size={size * 0.8} />;
}

const tokensLabel = (n: number) => `${compact(n)} tokens`;

/** AI Guard → Discover: the AI services found in LLM spans, the model providers they call, and which ones AI Guard protects. */
export function AiGuardDiscoverPage() {
  return (
    <TimeProvider initial="1w">
      <Discover />
    </TimeProvider>
  );
}

function Discover() {
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const q = useAiDiscover(r);
  const [panel, setPanel] = useState(true);
  const [zoom, setZoom] = useState<number | null>(null);
  const [canvasW, setCanvasW] = useState(0);
  const canvasRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setCanvasW(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, [q.data]);
  const [filter, setFilter] = useState<'all' | 'protected' | 'unprotected'>('all');
  const [search, setSearch] = useState('');
  const d = q.data;
  const mapW = useMemo(() => (d ? mapLayout(d.providers, d.services).width + 40 : 0), [d]);
  const fit = canvasW && mapW ? Math.max(0.5, Math.min(1, canvasW / mapW)) : 1;
  const scale = zoom ?? fit;
  const services = (d?.services ?? []).filter((s) => (filter === 'all' || (filter === 'protected') === s.protected) && s.service.toLowerCase().includes(search.trim().toLowerCase()));
  return (
    <div className="sec-page aig-page aig-discover">
      <div className="aig-filters">
        <span className="aig-grow" />
        <TimePicker compact />
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError || !d ? (
        <ErrorState error={q.error} />
      ) : !d.services.length ? (
        <Empty>
          <b>No AI services found.</b>
          <p className="muted">Services appear here from their LLM spans (OpenTelemetry GenAI, OpenLLMetry, OpenInference or the tracers' LLM Observability), and from AI Guard evaluations.</p>
        </Empty>
      ) : (
        <div className={`aig-discover-body${panel ? '' : ' wide'}`}>
          <div className="aig-canvas" ref={canvasRef}>
            <div className="aig-zoom">
              <button className="btn btn-icon" onClick={() => setZoom(Math.max(0.4, scale - 0.1))} aria-label="Zoom out">
                <Minus size={14} />
              </button>
              <button className="btn btn-icon" onClick={() => setZoom(null)} aria-label="Fit">
                <Maximize2 size={14} />
              </button>
              <button className="btn btn-icon" onClick={() => setZoom(Math.min(1.6, scale + 0.1))} aria-label="Zoom in">
                <Plus size={14} />
              </button>
              <button className={`btn btn-icon${panel ? ' active' : ''}`} onClick={() => setPanel(!panel)} aria-label="Toggle panel">
                <PanelRight size={14} />
              </button>
            </div>
            <div className="aig-map" style={{ transform: `scale(${scale})` }}>
              <ServiceMap providers={d.providers} services={d.services} />
            </div>
          </div>
          {panel && (
            <aside className="aig-discover-side">
              <h4>Model Providers</h4>
              <div className="aig-provider-grid">
                {PROVIDERS.map(([k, name]) => {
                  const p = d.providers.find((x) => x.provider === k);
                  return (
                    <div
                      key={k}
                      className={`aig-provider-cell${p ? '' : ' unused'}`}
                      title={p ? `${name}: ${tokensLabel(p.tokens)}, ${p.services.length} service${p.services.length === 1 ? '' : 's'}` : `${name}: not used`}
                    >
                      <span className="aig-provider-icon">
                        <ProviderLogo provider={k} size={20} />
                      </span>
                      <span>{name}</span>
                    </div>
                  );
                })}
              </div>
              <h4>AI Services</h4>
              <label className="sec-search">
                <Search size={14} />
                <input placeholder="Search services" value={search} onChange={(e) => setSearch(e.target.value)} />
              </label>
              <div className="aig-toggle-row">
                <span className="muted">{services.length} items</span>
                <div className="aig-toggle">
                  {(['all', 'protected', 'unprotected'] as const).map((f) => (
                    <button key={f} className={filter === f ? 'active' : ''} onClick={() => setFilter(f)}>
                      {f[0].toUpperCase() + f.slice(1)}
                    </button>
                  ))}
                </div>
              </div>
              <table className="mon-table syn-table aig-table">
                <thead>
                  <tr>
                    <th>SERVICE</th>
                    <th className="num">TOKENS</th>
                    <th>PROTECTED</th>
                    <th>SECURITY</th>
                  </tr>
                </thead>
                <tbody>
                  {services.map((s) => (
                    <tr key={s.service}>
                      <td>
                        <span className="aig-svc-name">
                          <i className={`aig-svc-dot${s.protected ? ' on' : ''}`}>
                            <Globe size={11} />
                          </i>
                          {s.protected ? <Link to={`/security/ai-guard?service=${encodeURIComponent(s.service)}`}>{s.service}</Link> : s.service}
                        </span>
                      </td>
                      <td className="num">{compact(s.tokens)}</td>
                      <td>{s.protected ? <ShieldCheck size={15} className="aig-ok" aria-label="Protected" /> : <ShieldOff size={15} className="aig-bad" aria-label="Not protected" />}</td>
                      <td>
                        <Security s={s} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </aside>
          )}
        </div>
      )}
    </div>
  );
}

function Security({ s }: { s: AiService }) {
  if (!s.unsafeCount && !s.sensitive) return <span className="muted">—</span>;
  return (
    <span className="aig-sec">
      {s.unsafeCount > 0 && (
        <Link to={`/security/ai-guard?service=${encodeURIComponent(s.service)}&assessment=unsafe`} title={`${s.unsafeCount} unsafe steps`}>
          <ShieldAlert size={14} className="aig-bad" /> {compact(s.unsafeCount)}
        </Link>
      )}
      {s.sensitive > 0 && (
        <Link to={`/security/ai-guard?service=${encodeURIComponent(s.service)}&q=${encodeURIComponent('@ai_guard.sds:true')}`} title={`${s.sensitive} steps with sensitive data`}>
          <ShieldAlert size={14} className="aig-warn" /> {compact(s.sensitive)}
        </Link>
      )}
    </span>
  );
}

const COL = 124;
const CELL = 128;
const PER_ROW = 4;
const SHOWN = 9;

/** Providers on top, then the unprotected and protected services in their boxes, linked by edges. */
/** Where everything goes: providers left to right (those of unprotected services first, so edges cross less),
 * then the unprotected and protected services in their boxes, centered under them. */
function mapLayout(allProviders: AiProvider[], services: AiService[]) {
  const share = (p: AiProvider) => services.filter((s) => s.providers.includes(p.provider) && s.protected).length / Math.max(1, p.services.length);
  const providers = [...allProviders].sort((a, b) => share(a) - share(b) || b.tokens - a.tokens);
  const groups: [string, AiService[], boolean][] = [
    ['Not Protected by AI Guard', services.filter((s) => !s.protected), false],
    ['Protected by AI Guard', services.filter((s) => s.protected), true],
  ];
  const PROV_Y = 30;
  const BOX_Y = 170;
  const pos = new Map<string, { x: number; y: number }>();
  const boxes: { title: string; x: number; y: number; w: number; h: number; ok: boolean; n: number; more: number }[] = [];
  let bx = 0;
  for (const [title, list, ok] of groups) {
    if (!list.length) continue;
    const shown = list.slice(0, list.length > SHOWN ? SHOWN - 1 : SHOWN);
    const cells = shown.length + (list.length > shown.length ? 1 : 0);
    const cols = Math.min(ok ? 2 : PER_ROW, cells);
    const rows = Math.ceil(cells / cols);
    const w = cols * CELL + 20;
    shown.forEach((s, i) => pos.set(s.service, { x: bx + 10 + (i % cols) * CELL + CELL / 2, y: BOX_Y + 50 + Math.floor(i / cols) * 92 + 30 }));
    boxes.push({ title, x: bx, y: BOX_Y, w, h: rows * 92 + 60, ok, n: list.length, more: list.length - shown.length });
    bx += w + 30;
  }
  const boxesW = Math.max(0, bx - 30);
  const provW = providers.length * COL;
  const width = Math.max(provW, boxesW) + 20;
  // Center the narrower row under the wider one.
  const shift = (width - boxesW) / 2;
  for (const b of boxes) b.x += shift;
  for (const [k, v] of pos) pos.set(k, { x: v.x + shift, y: v.y });
  const provX = (i: number) => (width - provW) / 2 + i * COL + COL / 2;
  const height = Math.max(...boxes.map((b) => b.y + b.h), 300) + 20;
  return { providers, pos, boxes, provX, width, height, PROV_Y };
}

function ServiceMap({ providers: all, services }: { providers: AiProvider[]; services: AiService[] }) {
  const { providers, pos, boxes, provX, width: canvasW, height, PROV_Y } = mapLayout(all, services);
  return (
    <div className="aig-map-inner" style={{ width: canvasW, height }}>
      <svg width={canvasW} height={height} className="aig-edges">
        {services.flatMap((s) =>
          s.providers.map((p) => {
            const i = providers.findIndex((x) => x.provider === p);
            const to =
              pos.get(s.service) ??
              (() => {
                const b = boxes.find((x) => x.ok === s.protected);
                return b ? { x: b.x + b.w - 70, y: b.y + b.h - 40 } : undefined;
              })();
            if (i < 0 || !to) return null;
            const from = { x: provX(i), y: PROV_Y + 46 };
            const mid = (from.y + to.y - 30) / 2;
            return <path key={`${s.service}-${p}`} d={`M${from.x},${from.y} C${from.x},${mid} ${to.x},${mid} ${to.x},${to.y - 32}`} className={s.protected ? 'ok' : 'bad'} />;
          }),
        )}
      </svg>
      {providers.map((p, i) => (
        <div key={p.provider} className="aig-node aig-node-provider" style={{ left: provX(i), top: PROV_Y }}>
          <span className="aig-node-icon">
            <ProviderLogo provider={p.provider} />
          </span>
          <b>{providerName(p.provider)}</b>
          <span className="muted">{tokensLabel(p.tokens)}</span>
        </div>
      ))}
      {boxes.map((b) => (
        <div key={b.title} className={`aig-box-group${b.ok ? ' ok' : ''}`} style={{ left: b.x, top: b.y, width: b.w, height: b.h }}>
          <div className="aig-box-title">
            {b.ok ? <ShieldCheck size={14} /> : <ShieldOff size={14} />} {b.title}{' '}
            <span className="muted">
              — {b.n} AI Service{b.n === 1 ? '' : 's'}
            </span>
          </div>
          {b.more > 0 && (
            <div className="aig-node aig-node-more" style={{ left: b.w - 80, top: b.h - 72 }}>
              <span className="aig-node-icon">
                <Plus size={16} />
              </span>
              <span className="muted">{b.more} other services</span>
            </div>
          )}
        </div>
      ))}
      {services.map((s) => {
        const p = pos.get(s.service);
        if (!p) return null;
        return (
          <div
            key={s.service}
            className={`aig-node aig-node-service${s.protected ? ' ok' : ''}`}
            style={{ left: p.x, top: p.y - 30 }}
            title={`${s.service}: ${s.models.join(', ') || 'no model reported'}`}
          >
            <span className="aig-node-icon">
              {s.models.length ? <Globe size={16} /> : <Cog size={16} />}
              {s.unsafeCount > 0 && <i className="aig-badge">{compact(s.unsafeCount)}</i>}
            </span>
            <b>{s.service}</b>
            <span className="muted">{tokensLabel(s.tokens)}</span>
          </div>
        );
      })}
    </div>
  );
}
