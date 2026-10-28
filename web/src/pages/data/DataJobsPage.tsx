import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useSearchParams } from 'react-router-dom';
import { ArrowRight, Workflow, PanelLeft, X } from 'lucide-react';
import { searchDataJobs, type DataJob, type JobSearchPage, type JobStatus } from '../../api/databases/client';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { SetupOptions } from '../setup/SetupOptions';
import { JOB_SETUP } from './dataSetup';
import { formatDuration } from './format';
import './data.css';

const FACET_LABELS: Record<string, string> = { framework: 'Framework', status: 'Status', env: 'Env', cluster: 'Cluster', host: 'Host' };
const STATUS_LABELS: Record<JobStatus, string> = { failed: 'Failed', running: 'Running', succeeded: 'Succeeded', unknown: 'Unknown' };
const SOURCES: { name: string; note: string; logo: string }[] = [
  { name: 'Spark', note: 'Spark check', logo: 'spark' },
  { name: 'Databricks', note: 'Spark check on the driver', logo: 'databricks' },
  { name: 'Airflow', note: 'StatsD metrics', logo: 'apacheairflow' },
  { name: 'dbt', note: 'not supported yet', logo: 'dbt' },
];

/** Data Observability → Data Jobs: Airflow DAGs and Spark applications with their runs. */
export function DataJobsPage() {
  const { range } = useTime();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const [setup, setSetup] = useState(false);
  const page = useQuery({ queryKey: ['data-jobs', range, query], queryFn: () => searchDataJobs({ range, query }), placeholderData: keepPreviousData });
  if (page.data && !page.data.enabled)
    return (
      <div className="dbm-page">
        <div className="dbm-body">
          <JobsHero onSetup={() => setSetup(true)} />
        </div>
        {setup && <JobsSetup onClose={() => setSetup(false)} />}
      </div>
    );
  return (
    <div className="dbm-page">
      <header className="product-header">
        <div className="product-title">
          <Workflow size={20} />
          <span className="dbm-crumb">Data Observability /</span> Data Jobs
        </div>
        <div className="dbm-header-right">
          <button className="btn btn-sm" onClick={() => setSetup(true)}>
            Setup
          </button>
          <TimePicker />
        </div>
      </header>
      <div className="dbm-body">
        {page.isError ? (
          <ErrorState error={page.error} onRetry={() => page.refetch()} />
        ) : !page.data ? (
          <Loading label="Loading data jobs…" />
        ) : (
          <JobsList page={page.data} query={query} onQuery={(q) => setParams(q ? { q } : {}, { replace: true })} />
        )}
      </div>
      {setup && <JobsSetup onClose={() => setSetup(false)} />}
    </div>
  );
}

function JobsSetup({ onClose }: { onClose: () => void }) {
  return (
    <Modal label="Data Jobs setup" onClose={onClose} width={860} height="auto">
      <div className="dbm-modal">
        <header>
          <h2>Send data job metrics to ObserveCraft</h2>
          <button className="icon-btn" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <SetupOptions options={JOB_SETUP} />
      </div>
    </Modal>
  );
}

/** The demo rows in the hero's preview: fictional jobs of the e-commerce demo. */
const PREVIEW: { name: string; framework: string; status: JobStatus; width: number }[] = [
  { name: 'orders_daily_etl', framework: 'Airflow', status: 'failed', width: 72 },
  { name: 'catalog-enrichment', framework: 'Spark', status: 'running', width: 48 },
  { name: 'inventory_snapshot', framework: 'Airflow', status: 'succeeded', width: 86 },
  { name: 'billing-reconciliation', framework: 'Spark', status: 'succeeded', width: 34 },
  { name: 'storefront_sessions_rollup', framework: 'Airflow', status: 'succeeded', width: 58 },
];

function JobsHero({ onSetup }: { onSetup: () => void }) {
  return (
    <section className="dj-hero">
      <div className="dj-hero-text">
        <div className="dj-hero-kicker">
          <span className="dj-hero-icon">
            <Workflow size={22} />
          </span>
          Data Observability
        </div>
        <h2>
          Reliable pipelines,
          <br />
          fresh data,
          <br />
          no surprises.
        </h2>
        <p>Follow your Airflow DAGs and Spark applications: which runs failed, which ran late and which tasks broke, from the metrics the Datadog Agent already collects.</p>
        <ul className="dj-sources">
          {SOURCES.map((x) => (
            <li key={x.name} title={x.note}>
              <strong>
                <span className="dj-source-logo">
                  <IntegrationLogo logo={x.logo} name={x.name} size={26} />
                </span>
                {x.name}
              </strong>
              <span>{x.note}</span>
            </li>
          ))}
        </ul>
        <button className="dj-hero-cta" onClick={onSetup}>
          Set up Data Jobs <ArrowRight size={18} />
        </button>
      </div>
      <div className="dj-hero-preview" aria-hidden="true">
        <div className="dj-preview-head">
          <Workflow size={16} /> Data Jobs
        </div>
        <div className="dj-preview-rows">
          {PREVIEW.map((r) => (
            <div key={r.name} className="dj-preview-row">
              <span className={`dbm-status dbm-status-${r.status}`}>{STATUS_LABELS[r.status]}</span>
              <span className="dj-preview-name">
                <strong>{r.name}</strong>
                <span>{r.framework}</span>
              </span>
              <span className="dj-preview-bar">
                <span className={`dbm-status-${r.status}`} style={{ width: `${r.width}%` }} />
              </span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function JobsList({ page, query, onQuery }: { page: JobSearchPage; query: string; onQuery: (q: string) => void }) {
  const [showFacets, setShowFacets] = useState(true);
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo(() => page.facets.map((f) => ({ key: f.key, label: FACET_LABELS[f.key] ?? f.key, group: '', values: f.values })), [page.facets]);
  return (
    <>
      <div className="dbm-query">
        <QueryBar value={query} onChange={onQuery} placeholder="Filter jobs, such as framework:airflow status:failed env:prod" />
      </div>
      <div className="dbm-explorer">
        {showFacets && (
          <div className="dbm-facets">
            <FacetPanel facets={facets} terms={terms} onChange={(next: Term[]) => onQuery(formatQuery(next))} valueSearch defaultOpen={['status', 'framework', 'env']} verified={[]} />
          </div>
        )}
        <div className="dbm-results">
          <div className="dbm-results-head">
            <button className="icon-btn" onClick={() => setShowFacets((s) => !s)} title={showFacets ? 'Hide facets' : 'Show facets'}>
              <PanelLeft size={16} />
            </button>
            <strong>
              {page.jobs.length.toLocaleString()} {page.jobs.length === 1 ? 'job' : 'jobs'}
            </strong>
          </div>
          <div className="dbm-table-wrap">
            <table className="dbm-table">
              <thead>
                <tr>
                  <th>STATUS</th>
                  <th>JOB</th>
                  <th>FRAMEWORK</th>
                  <th>ENV</th>
                  <th>CLUSTER</th>
                  <th className="num">SUCCEEDED</th>
                  <th className="num">FAILED</th>
                  <th className="num">RUNNING</th>
                  <th className="num">AVG DURATION</th>
                  <th className="num">LAST DURATION</th>
                  <th className="num">FAILED TASKS</th>
                  <th>LAST SEEN</th>
                </tr>
              </thead>
              <tbody>
                {page.jobs.map((j: DataJob) => (
                  <tr key={j.id} className={j.status === 'failed' ? 'dbm-row-failed' : ''}>
                    <td>
                      <span className={`dbm-status dbm-status-${j.status}`}>{STATUS_LABELS[j.status]}</span>
                    </td>
                    <td className="mono">{j.name}</td>
                    <td>
                      <span className="dj-framework">
                        <IntegrationLogo logo={j.framework === 'airflow' ? 'apacheairflow' : 'spark'} name={j.framework} size={16} />
                        {j.framework === 'airflow' ? 'Airflow DAG' : 'Spark application'}
                      </span>
                    </td>
                    <td>{j.env ?? '—'}</td>
                    <td>{j.cluster ?? '—'}</td>
                    <td className="num">{j.succeeded.toLocaleString()}</td>
                    <td className={`num${j.failed ? ' dbm-warn' : ''}`}>{j.failed.toLocaleString()}</td>
                    <td className="num">{j.running.toLocaleString()}</td>
                    <td className="num">{formatDuration(j.avgDurationMs)}</td>
                    <td className="num">{formatDuration(j.lastDurationMs)}</td>
                    <td className={`num${j.failedTasks ? ' dbm-warn' : ''}`}>{j.failedTasks.toLocaleString()}</td>
                    <td>{timeAgo(j.lastSeen)}</td>
                  </tr>
                ))}
                {page.jobs.length === 0 && (
                  <tr>
                    <td colSpan={12} className="dbm-empty-row">
                      No jobs match this query.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </>
  );
}
