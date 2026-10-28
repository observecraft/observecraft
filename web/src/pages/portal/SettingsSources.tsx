import type { ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Activity, ArrowRight, Boxes, Container, FileText, MonitorSmartphone, Network, Plus, Radar } from 'lucide-react';
import type { PortalConnector } from '../../api/portal/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { useSources } from './homeQueries';

interface TelemetryCard {
  key: string;
  title: string;
  icon: ReactNode;
  text: string;
  to?: string;
  cta?: string;
  ribbon?: string;
}

const TELEMETRY: TelemetryCard[] = [
  {
    key: 'apm',
    title: 'APM',
    icon: <Activity size={18} />,
    text: 'Every service that sends traces becomes an entity, with its dependencies and deployments.',
    to: '/apm',
    cta: 'Go to APM',
    ribbon: 'Popular choice',
  },
  { key: 'logs', title: 'Logs', icon: <FileText size={18} />, text: 'Services named in your logs are added, even before they are traced.', to: '/logs', cta: 'Go to Logs' },
  {
    key: 'network',
    title: 'Cloud Network',
    icon: <Network size={18} />,
    text: 'Services seen talking to each other on the network are discovered from flows.',
    to: '/network',
    cta: 'Go to Cloud Network',
  },
  {
    key: 'containers',
    title: 'Container',
    icon: <Container size={18} />,
    text: 'Workloads running in containers and Kubernetes are mapped to services.',
    to: '/orchestration/explorer',
    cta: 'Go to Containers',
  },
  { key: 'rum', title: 'RUM', icon: <MonitorSmartphone size={18} />, text: 'Frontend applications from real user monitoring.' },
  { key: 'usm', title: 'Universal Service Monitoring', icon: <Radar size={18} />, text: 'Services detected from traffic without instrumentation.' },
];

interface ExternalCard {
  key: string;
  title: string;
  logo: string;
  text: string;
  provider?: 'github' | 'gitlab';
  ribbon?: string;
}

const EXTERNAL: ExternalCard[] = [
  { key: 'github', title: 'GitHub', logo: 'github', provider: 'github', ribbon: 'Recommended', text: 'Link repositories and code owners to services, and see pull requests on the portal home.' },
  { key: 'gitlab', title: 'GitLab', logo: 'gitlab', provider: 'gitlab', text: 'Link projects to services and see merge requests on the portal home.' },
  { key: 'terraform', title: 'Terraform', logo: 'terraform', text: 'Declare entities next to the infrastructure that runs them.' },
  { key: 'servicenow', title: 'ServiceNow', logo: 'tile:servicenow', text: 'Import services and their owners from your CMDB.' },
  { key: 'backstage', title: 'Backstage', logo: 'backstage', text: 'Import the entities of catalog-info.yaml files.' },
];

function Status({ state }: { state: 'active' | 'setup' | 'unavailable' }) {
  const text = state === 'active' ? 'Active' : state === 'setup' ? 'Setup Required' : 'Not available';
  return <span className={`ps-chip ps-chip-${state}`}>{text}</span>;
}

/** The card header: the source on the left, linked to the catalog on the right. */
function Pair({ left }: { left: ReactNode }) {
  return (
    <div className="ps-pair" aria-hidden="true">
      <span className="ps-pair-icon">{left}</span>
      <span className="ps-pair-line" />
      <span className="ps-pair-icon ps-pair-catalog">
        <Boxes size={18} />
      </span>
    </div>
  );
}

function connectorState(c: PortalConnector | undefined): 'active' | 'setup' {
  return c?.configured ? 'active' : 'setup';
}

/** Settings → Entities Sources: where catalog entities come from. */
export function SettingsSources() {
  const navigate = useNavigate();
  const q = useSources();
  return (
    <div className="ps-sources">
      <div className="ps-hero">
        <h1>Build your software catalog</h1>
        <p>
          The catalog lists what your teams build and run (services, frontends, data pipelines) with their owners, documentation and health. Entities are discovered from your telemetry, imported from
          the tools you already use, or added by hand.
        </p>
        <span className="ps-hero-step">Start by adding entities</span>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.error ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <h2 className="ps-h2">Use telemetry to discover entities</h2>
          <div className="ps-cards">
            {TELEMETRY.map((t) => {
              const signal = q.data!.signals.find((s) => s.key === t.key);
              const state = !t.to ? 'unavailable' : signal?.active ? 'active' : 'setup';
              return (
                <div key={t.key} className={`ps-card${state === 'unavailable' ? ' ps-card-off' : ''}`}>
                  {t.ribbon && <span className="ps-ribbon">{t.ribbon}</span>}
                  <Pair left={t.icon} />
                  <strong>{t.title}</strong>
                  <p>{t.text}</p>
                  <div className="ps-card-foot">
                    <Status state={state} />
                    {t.to && (
                      <button className="btn btn-sm" onClick={() => navigate(t.to!)}>
                        {t.cta} <ArrowRight size={13} />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          <h2 className="ps-h2">Use external sources</h2>
          <div className="ps-cards">
            {EXTERNAL.map((e) => {
              const connector = e.provider ? q.data!.connectors.find((c) => c.provider === e.provider) : undefined;
              return (
                <div key={e.key} className={`ps-card${e.provider ? '' : ' ps-card-off'}`}>
                  {e.ribbon && <span className="ps-ribbon">{e.ribbon}</span>}
                  <Pair left={<IntegrationLogo logo={e.logo} name={e.title} size={18} />} />
                  <strong>{e.title}</strong>
                  <p>{e.text}</p>
                  <div className="ps-card-foot">
                    {e.provider ? <Status state={connectorState(connector)} /> : <Status state="unavailable" />}
                    {e.provider ? (
                      <button className="btn btn-sm" onClick={() => navigate('/portal/settings/configurations')}>
                        Manage Source
                      </button>
                    ) : (
                      <span className="ps-soon">soon</span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          <h2 className="ps-h2">Create entities manually</h2>
          <div className="ps-manual">
            <div>
              <strong>Add manually</strong>
              <p className="sc-muted">Describe an entity the telemetry doesn't show yet: its owner, links and dependencies.</p>
            </div>
            <Link to="/portal/catalog/new" className="btn btn-primary">
              <Plus size={14} /> Add an Entity
            </Link>
          </div>
        </>
      )}
    </div>
  );
}
