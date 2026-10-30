import { Link } from 'react-router-dom';
import { Modal } from '../../../components/ui/Modal';
import { useMetricCatalog } from '../queries';

/** The metrics a widget reads: kind, unit, description and where to explore them. */
export function MetricsInfoModal({ title, metrics, onClose }: { title: string; metrics: string[]; onClose: () => void }) {
  const catalog = useMetricCatalog();
  return (
    <Modal onClose={onClose} label="Metrics info" width={720} height="auto">
      <div className="iw-metrics-info">
        <h3>Metrics in {title || 'this widget'}</h3>
        {metrics.map((name) => {
          const entry = catalog.data?.find((m) => m.name === name);
          return (
            <section key={name}>
              <div className="iw-metrics-info-head">
                <span className="mono">{name}</span>
                {entry?.kind && <span className="tag-chip">{entry.kind}</span>}
                {entry?.unit && <span className="tag-chip">{entry.unit}</span>}
              </div>
              <p>{entry?.description || (catalog.isPending ? 'Loading…' : entry ? 'No description.' : 'This metric has not been reported yet.')}</p>
              {entry && entry.tags.length > 0 && (
                <p className="muted-text">
                  Tags: {entry.tags.slice(0, 12).join(', ')}
                  {entry.tags.length > 12 ? ` +${entry.tags.length - 12}` : ''}
                </p>
              )}
              <div className="iw-metrics-info-links">
                <Link to={`/metrics?metric=${encodeURIComponent(name)}`} onClick={onClose}>
                  Open in Metrics Explorer
                </Link>
              </div>
            </section>
          );
        })}
      </div>
    </Modal>
  );
}
