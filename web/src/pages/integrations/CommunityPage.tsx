import { useCallback, useDeferredValue } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { CategoryChips, SearchBox } from './CatalogFilters';
import { IntegrationCard } from './IntegrationCard';
import { IntegrationModal } from './IntegrationModal';
import { useCatalog } from './queries';

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
    <svg className="cm-art" viewBox="0 0 640 220" preserveAspectRatio="xMaxYMid slice" aria-hidden>
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

/** How to install community integrations and build your own (Agent checks), and bring their assets here. */
const GUIDE = 'https://github.com/observecraft/observecraft/blob/main/docs/integrations.md';

/**
 * Integrations → Community: integrations-extras' community and partner integrations (Agent checks and log
 * integrations), the same catalog entries, cards and detail modal as the Integration Catalog.
 */
export function CommunityPage() {
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const category = params.get('category');
  const openId = params.get('integration');
  const setParam = useCallback(
    (key: string, value: string | null) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          if (!value) next.delete(key);
          else next.set(key, value);
          return next;
        },
        { replace: key !== 'integration' },
      ),
    [setParams],
  );
  const catalog = useCatalog({ query: useDeferredValue(query), status: 'all', category, repository: 'integrations-extras' });
  const items = catalog.data?.integrations ?? [];

  return (
    <div className="int-page cm-page">
      <section className="cm-hero">
        <div>
          <h1>Integrations built by the community and partners</h1>
          <p>Agent checks and log integrations from DataDog/integrations-extras, with their dashboards, log pipelines and monitors. They report through the Datadog Agent like any other.</p>
        </div>
        <HeroArt />
      </section>

      <div className="int-toolbar">
        <SearchBox value={query} onChange={(v) => setParam('q', v)} placeholder="Search community integrations" />
      </div>
      <CategoryChips value={category} onChange={(c) => setParam('category', c)} />

      <section className="int-group">
        <h2>All Community Integrations</h2>
        {catalog.isPending ? (
          <Loading label="Loading integrations…" />
        ) : catalog.isError ? (
          <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />
        ) : items.length === 0 ? (
          <Empty>
            No community integrations match “{query}”{category ? ` in ${category}` : ''}.
          </Empty>
        ) : (
          <div className="int-grid">
            {items.slice(0, 2).map((i) => (
              <IntegrationCard key={i.id} item={i} onOpen={(id) => setParam('integration', id)} />
            ))}
            {!query && !category && (
              <a className="int-card cm-contribute" href={GUIDE} target="_blank" rel="noreferrer">
                <strong>Build an integration</strong>
                <span className="btn btn-primary">Read the guide</span>
              </a>
            )}
            {items.slice(2).map((i) => (
              <IntegrationCard key={i.id} item={i} onOpen={(id) => setParam('integration', id)} />
            ))}
          </div>
        )}
      </section>

      {openId && <IntegrationModal id={openId} onClose={() => setParam('integration', null)} onCategory={(c) => setParams({ category: c })} />}
    </div>
  );
}
