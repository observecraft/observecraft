import { useCallback, useDeferredValue } from 'react';
import { useSearchParams } from 'react-router-dom';
import { CircleHelp } from 'lucide-react';
import type { IntegrationStatus, IntegrationSummary } from '../../api/integrations/types';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select, type Option } from '../../components/ui/Dropdown';
import { CategoryChips, SearchBox } from './CatalogFilters';
import { IntegrationCard } from './IntegrationCard';
import { IntegrationModal } from './IntegrationModal';
import { useCatalog } from './queries';

const STATUSES: Option<IntegrationStatus | 'all'>[] = [
  { value: 'all', label: 'All Statuses' },
  { value: 'detected', label: 'Detected' },
  { value: 'installed', label: 'Installed' },
  { value: 'warning', label: 'Needs Attention' },
  { value: 'available', label: 'Available' },
];

const SECTIONS: { title: string; hint?: string; match: (s: IntegrationStatus) => boolean }[] = [
  { title: 'Autodetected Integrations', hint: 'Software running on your hosts that has an integration but is not configured yet.', match: (s) => s === 'detected' },
  { title: 'Installed', match: (s) => s === 'installed' || s === 'warning' },
  { title: 'Available', match: (s) => s === 'available' },
];

export function CatalogPage() {
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const status = (params.get('status') as IntegrationStatus | null) ?? 'all';
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

  // Typing stays responsive; the search request follows the deferred value.
  const deferredQuery = useDeferredValue(query);
  const catalog = useCatalog({ query: deferredQuery, status, category });
  const items = catalog.data?.integrations ?? [];

  const open = useCallback((id: string) => setParam('integration', id), [setParam]);

  return (
    <div className="int-page">
      <div className="int-toolbar">
        <SearchBox value={query} onChange={(v) => setParam('q', v)} placeholder="Search integrations" />
        <Select value={status} options={STATUSES} onChange={(v) => setParam('status', v === 'all' ? null : v)} triggerClassName="dd-trigger int-status" align="right" />
      </div>
      <CategoryChips value={category} onChange={(c) => setParam('category', c)} />

      {catalog.isPending ? (
        <Loading label="Loading integrations…" />
      ) : catalog.isError ? (
        <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />
      ) : items.length === 0 ? (
        <Empty>No integrations match “{query}”{category ? ` in ${category}` : ''}.</Empty>
      ) : (
        <div className={catalog.isPlaceholderData ? 'is-stale' : undefined}>
          {SECTIONS.map((s) => {
            const list: IntegrationSummary[] = items.filter((i) => s.match(i.status));
            if (list.length === 0) return null;
            return (
              <section key={s.title} className="int-group">
                <h2>
                  {s.title}
                  {s.hint && (
                    <span title={s.hint} className="int-hint">
                      <CircleHelp size={16} />
                    </span>
                  )}
                </h2>
                <div className="int-grid">
                  {list.map((i) => (
                    <IntegrationCard key={i.id} item={i} onOpen={open} />
                  ))}
                </div>
              </section>
            );
          })}
        </div>
      )}

      {openId && <IntegrationModal id={openId} onClose={() => setParam('integration', null)} onCategory={(c) => setParams({ category: c })} />}
    </div>
  );
}
