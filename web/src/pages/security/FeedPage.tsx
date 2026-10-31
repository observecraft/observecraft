import { useState } from 'react';
import { AlertTriangle, CheckCircle2, ExternalLink, Flame, ShieldAlert } from 'lucide-react';
import { Select } from '../../components/ui/Dropdown';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { useResearchFeed } from './queries';

const TYPES: Record<string, [string, string]> = {
  exploited_vulnerability: ['Exploited Vulnerability', 'exploited'],
  emerging_vulnerability: ['Emerging Vulnerability', 'emerging'],
};
const PRODUCTS = ['Code Security', 'App & API Protection', 'Cloud SIEM'];

/** Security → Research Feed: vulnerabilities worth knowing about, and whether they touch you. */
export function FeedPage() {
  const q = useResearchFeed();
  const [product, setProduct] = useState('');
  const [type, setType] = useState('');
  const [impacted, setImpacted] = useState(false);
  const [sort, setSort] = useState<'date' | 'impact'>('date');
  const items = (q.data ?? [])
    .filter((i) => (!product || i.products.includes(product)) && (!type || i.articleType === type) && (!impacted || i.impacted))
    .sort((a, b) => (sort === 'impact' ? Number(b.impacted) - Number(a.impacted) || b.publishedAt - a.publishedAt : b.publishedAt - a.publishedAt));
  return (
    <div className="sec-page sec-feed">
      <div className="sec-feed-head">
        <div>
          <h2 className="sec-h">Research Feed</h2>
          <p className="muted">Vulnerabilities exploited in the wild and new advisories for the packages you use, and whether they affect you.</p>
        </div>
      </div>
      <div className="sec-filters">
        <Select label="Product" value={product} onChange={setProduct} options={[{ value: '', label: 'All' }, ...PRODUCTS.map((p) => ({ value: p, label: p }))]} />
        <Select label="Article type" value={type} onChange={setType} options={[{ value: '', label: 'All' }, ...Object.entries(TYPES).map(([v, [l]]) => ({ value: v, label: l }))]} />
        <div className="sec-seg">
          <span>Exposure</span>
          <button className={!impacted ? 'active' : ''} onClick={() => setImpacted(false)}>
            All
          </button>
          <button className={impacted ? 'active' : ''} onClick={() => setImpacted(true)}>
            Impacted
          </button>
        </div>
        <span className="syn-grow" />
        <Select
          label="Sort by"
          value={sort}
          onChange={(v) => setSort(v as 'date' | 'impact')}
          options={[
            { value: 'date', label: 'Date' },
            { value: 'impact', label: 'Impacted first' },
          ]}
        />
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !items.length ? (
        <Empty>
          <b>No articles.</b>
          <p className="muted">The catalog of exploited vulnerabilities is downloaded every six hours when the server can reach it (OC_KEV_URL).</p>
        </Empty>
      ) : (
        <ol className="sec-feed-list">
          {items.map((i) => {
            const [label, kind] = TYPES[i.articleType] ?? [i.articleType, 'emerging'];
            return (
              <li key={i.id} className={i.impacted ? 'impacted' : ''}>
                <div className={`sec-feed-tile ${kind}`}>
                  {kind === 'exploited' ? <Flame size={22} /> : <ShieldAlert size={22} />}
                  <span>{label}</span>
                </div>
                <div className="sec-feed-text">
                  <h3>
                    {i.url ? (
                      <a href={i.url} target="_blank" rel="noreferrer noopener">
                        {i.title} <ExternalLink size={13} />
                      </a>
                    ) : (
                      i.title
                    )}
                  </h3>
                  <div className="sec-tags">
                    <span className={`sec-type ${kind}`}>{label.toUpperCase()}</span>
                    {i.ransomware && <span className="tag-chip sec-ransom">Used in ransomware</span>}
                    {i.severity && <span className={`tag-chip sev-text-${i.severity}`}>{i.severity}</span>}
                    {i.products.map((p) => (
                      <span key={p} className="tag-chip">
                        {p}
                      </span>
                    ))}
                    <span className="muted">
                      {i.source} · {new Date(i.publishedAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}
                    </span>
                  </div>
                  <p>{i.summary}</p>
                  {i.requiredAction && (
                    <p className="muted sec-feed-action">
                      {i.requiredAction}
                      {i.dueDate && ` Due ${i.dueDate}.`}
                    </p>
                  )}
                </div>
                <div className="sec-feed-impact">
                  {i.impacted ? (
                    <span className="sec-impacted" title={i.impactedResources.join(', ')}>
                      <AlertTriangle size={14} /> IMPACTED
                      {i.impactedResources.length > 0 && <em>{i.impactedResources.slice(0, 3).join(', ')}</em>}
                    </span>
                  ) : (
                    <span className="sec-not-detected">
                      <CheckCircle2 size={14} /> NOT DETECTED
                    </span>
                  )}
                </div>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
}
