import { useDeferredValue, useState } from 'react';
import { X } from 'lucide-react';
import type { MarketplaceApp } from '../../api/integrations/types';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { CategoryChips, SearchBox } from './CatalogFilters';
import { IntegrationLogo } from './IntegrationLogo';
import { useMarketplace } from './queries';

/** Decorative isometric blocks for the hero — brand purples, no external artwork. */
function HeroArt() {
  const block = (x: number, y: number, w: number, h: number, i: number) => (
    <g key={i} transform={`translate(${x} ${y})`}>
      <polygon points={`0,${h * 0.3} ${w * 0.5},0 ${w},${h * 0.3} ${w * 0.5},${h * 0.6}`} fill="#f2efff" />
      <polygon points={`0,${h * 0.3} ${w * 0.5},${h * 0.6} ${w * 0.5},${h * 1.6} 0,${h * 1.3}`} fill="#b9a6ff" />
      <polygon points={`${w * 0.5},${h * 0.6} ${w},${h * 0.3} ${w},${h * 1.3} ${w * 0.5},${h * 1.6}`} fill="#8f73ff" />
      {[0, 1, 2].map((k) => (
        <rect key={k} x={w * 0.12} y={h * (0.55 + k * 0.22)} width={w * 0.28} height={h * 0.08} fill="#5b3bd6" transform={`skewY(30)`} />
      ))}
    </g>
  );
  return (
    <svg className="mp-art" viewBox="0 0 640 220" preserveAspectRatio="xMaxYMid slice" aria-hidden>
      {block(40, 70, 120, 90, 0)}
      {block(170, 20, 140, 100, 1)}
      {block(330, 60, 120, 120, 2)}
      {block(470, 10, 150, 110, 3)}
      <rect x="250" y="8" width="8" height="40" fill="#e0218a" opacity="0.7" />
      <rect x="264" y="20" width="8" height="28" fill="var(--brand)" />
      <rect x="278" y="2" width="8" height="46" fill="#e0218a" opacity="0.7" />
    </svg>
  );
}

function AppCard({ app, onOpen }: { app: MarketplaceApp; onOpen: () => void }) {
  return (
    <button className="int-card mp-card" onClick={onOpen}>
      <span className="int-card-logo">
        <IntegrationLogo logo={app.logo} name={app.name} />
      </span>
      <span className="int-card-name">{app.name}</span>
      <span className="int-card-by">by {app.partner}</span>
      <span className="mp-offering">{app.offering}</span>
    </button>
  );
}

export function MarketplacePage() {
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<string | null>(null);
  const [open, setOpen] = useState<MarketplaceApp | null>(null);
  const apps = useMarketplace({ query: useDeferredValue(query), category });
  const list = apps.data ?? [];

  return (
    <div className="int-page mp-page">
      <section className="mp-hero">
        <div>
          <h1>Extend the power of ObserveCraft with apps</h1>
          <p>Marketplace is a collection of apps by our partners to extend the power of the ObserveCraft platform and improve workflows.</p>
        </div>
        <HeroArt />
      </section>

      <div className="int-toolbar">
        <SearchBox value={query} onChange={setQuery} placeholder="Search apps" />
      </div>
      <CategoryChips value={category} onChange={setCategory} />

      <section className="int-group">
        <h2>All Apps</h2>
        {apps.isPending ? (
          <Loading label="Loading apps…" />
        ) : apps.isError ? (
          <ErrorState error={apps.error} onRetry={() => apps.refetch()} />
        ) : list.length === 0 ? (
          <Empty>No apps match your search.</Empty>
        ) : (
          <div className={`int-grid${apps.isPlaceholderData ? ' is-stale' : ''}`}>
            {list.slice(0, 2).map((a) => (
              <AppCard key={a.id} app={a} onOpen={() => setOpen(a)} />
            ))}
            {list.slice(2).map((a) => (
              <AppCard key={a.id} app={a} onOpen={() => setOpen(a)} />
            ))}
          </div>
        )}
      </section>

      {open && (
        <Modal onClose={() => setOpen(null)} label={open.name} width={640} height="auto">
          <div className="mp-modal">
            <header className="im-header">
              <span className="im-logo">
                <IntegrationLogo logo={open.logo} name={open.name} size={30} />
              </span>
              <div className="im-title">
                <div>
                  <h2>{open.name}</h2>
                  <span className="im-badge badge-available">{open.offering.toUpperCase()}</span>
                </div>
                <p>by {open.partner}</p>
              </div>
              <div className="im-actions">
                <button className="icon-plain" onClick={() => setOpen(null)} title="Close">
                  <X size={22} />
                </button>
              </div>
            </header>
            <div className="im-text">
              <p>{open.description}</p>
              <p className="muted-text">Categories: {open.categories.join(', ')}</p>
              <button className="btn btn-primary" disabled title="Marketplace purchasing is not available yet">
                Request access
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
