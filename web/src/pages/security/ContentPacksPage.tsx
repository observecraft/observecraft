import { useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import { Check, Search, X } from 'lucide-react';
import type { ContentPackStatus } from '../../api/security/client';
import { Select } from '../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact, timeAgo } from '../../lib/format';
import { PackLogo, SeverityDot } from './common';
import { useContentPacks, useSetPack } from './queries';

const STATUSES: [string, string][] = [
  ['warning', 'Warning'],
  ['active', 'Active'],
  ['available', 'Available'],
  ['disabled', 'Disabled'],
];

/** Cloud SIEM → Content Packs: per log source, its detection rules and whether its logs arrive. */
export function ContentPacksPage() {
  const [params, setParams] = useSearchParams();
  const q = useContentPacks();
  const [search, setSearch] = useState('');
  const [category, setCategory] = useState('');
  const status = params.get('status') ?? '';
  const openId = params.get('pack');
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const packs = q.data!;
  const shown = packs.filter((p) => (!category || p.category === category) && (!search.trim() || `${p.name} ${p.description} ${p.query}`.toLowerCase().includes(search.trim().toLowerCase())));
  const categories = [...new Set(packs.map((p) => p.category))].sort();
  const open = packs.find((p) => p.id === openId);
  return (
    <div className="sec-page">
      <div className="sec-explorer-top">
        <h2 className="sec-h">Configure critical log sources to secure your environments</h2>
      </div>
      <p className="muted">
        Showing <b>{shown.length}</b> of <b>{packs.length}</b> content packs
      </p>
      <div className="sec-filters">
        <label className="sec-search">
          <Search size={14} />
          <input placeholder="Search content packs" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <Select label="Category" value={category} onChange={setCategory} options={[{ value: '', label: 'View all' }, ...categories.map((c) => ({ value: c, label: c }))]} />
      </div>
      <div className="siem-chips">
        {STATUSES.map(([v, l]) => (
          <button key={v} className={`siem-chip ${v}${status === v ? ' active' : ''}`} onClick={() => set('status', status === v ? null : v)}>
            {l} <b>{shown.filter((p) => p.status === v).length}</b>
          </button>
        ))}
      </div>
      {STATUSES.filter(([v]) => (!status || status === v) && shown.some((p) => p.status === v)).map(([v, l]) => (
        <section key={v} className="siem-pack-section">
          <h4>
            {l} <span className="muted">{shown.filter((p) => p.status === v).length} Content Packs</span>
          </h4>
          <div className="siem-packs">
            {shown
              .filter((p) => p.status === v)
              .map((p) => (
                <button key={p.id} className="siem-pack" onClick={() => set('pack', p.id)}>
                  <div className="siem-pack-art">
                    <span>
                      <PackLogo logo={p.logo} size={30} />
                    </span>
                  </div>
                  <small>CONTENT PACK · {p.category.toUpperCase()}</small>
                  <b>{p.name}</b>
                  <p>{p.description}</p>
                  <span className="siem-includes">This content pack includes:</span>
                  <ul>
                    <li>
                      {p.rules.length} detection rule{p.rules.length === 1 ? '' : 's'}
                    </li>
                    <li>
                      Logs: <span className="mono">{p.query}</span>
                    </li>
                    {p.signals7d > 0 && <li>{p.signals7d} signals in the last 7 days</li>}
                  </ul>
                  <span className={`siem-pack-status ${p.status}`}>
                    {p.status === 'active' ? <Check size={13} /> : null} {p.status.toUpperCase()}
                  </span>
                </button>
              ))}
          </div>
        </section>
      ))}
      {open && createPortal(<PackDrawer pack={open} onClose={() => set('pack', null)} />, document.body)}
    </div>
  );
}

function PackDrawer({ pack, onClose }: { pack: ContentPackStatus; onClose: () => void }) {
  const toggle = useSetPack();
  return (
    <div className="sec-panel-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sec-panel narrow" role="dialog" aria-label={pack.name}>
        <header className="sec-panel-head">
          <PackLogo logo={pack.logo} size={26} />
          <h2>{pack.name}</h2>
          <span className={`siem-pack-status inline ${pack.status}`}>{pack.status.toUpperCase()}</span>
          <span className="syn-grow" />
          <button className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <div className="siem-drawer-body">
          <p>{pack.description}</p>
          <label className="siem-toggle">
            <input type="checkbox" checked={pack.enabled} disabled={toggle.isPending} onChange={(e) => toggle.mutate({ id: pack.id, enabled: e.target.checked })} />
            <span>{pack.enabled ? 'On: its rules evaluate the logs' : 'Off: its rules are not evaluated'}</span>
          </label>
          <h4>Logs</h4>
          <p>
            {pack.logs24h > 0 ? (
              <>
                <b>{compact(pack.logs24h)}</b> logs in the last 24 hours (<span className="mono">{pack.query}</span>). <Link to={`/logs?query=${encodeURIComponent(pack.query)}`}>View logs</Link>
              </>
            ) : pack.lastLogAt ? (
              <>No logs in the last 24 hours; the last arrived {timeAgo(pack.lastLogAt)}.</>
            ) : (
              <>
                No logs with <span className="mono">{pack.query}</span> yet.
              </>
            )}
          </p>
          <h4>Set up the source</h4>
          <p>{pack.setup}</p>
          <h4>Detection rules</h4>
          <ul className="siem-rule-list">
            {pack.rules.map((r) => (
              <li key={r.id}>
                <SeverityDot severity={r.severity} />
                <Link to={`/security/siem/rules/${r.id}`}>{r.name}</Link>
                {!r.enabled && <span className="muted">disabled</span>}
              </li>
            ))}
          </ul>
        </div>
      </aside>
    </div>
  );
}
