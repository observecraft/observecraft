import { useSearchParams } from 'react-router-dom';
import { DnsAnalytics } from './DnsAnalytics';
import { NetworkAnalytics } from './NetworkAnalytics';

/** Network / DNS toggle of Network Analytics; each view keeps its own query and grouping. */
function ViewToggle({ dns }: { dns: boolean }) {
  const [, setParams] = useSearchParams();
  return (
    <div className="btn-group net-view-toggle">
      <button className={`btn${dns ? '' : ' btn-active'}`} onClick={() => setParams(new URLSearchParams(), { replace: true })}>
        Network
      </button>
      <button className={`btn${dns ? ' btn-active' : ''}`} onClick={() => setParams(new URLSearchParams({ view: 'dns' }), { replace: true })}>
        DNS
      </button>
    </div>
  );
}

/** Cloud Network Monitoring → Analytics. */
export function AnalyticsPage() {
  const [params] = useSearchParams();
  const dns = params.get('view') === 'dns';
  if (!dns) return <NetworkAnalytics toggle={<ViewToggle dns={false} />} />;
  return (
    <div className="net-page">
      <ViewToggle dns />
      <DnsAnalytics />
    </div>
  );
}
