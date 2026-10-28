import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useSearchParams } from 'react-router-dom';
import { Workflow, PanelLeft, X } from 'lucide-react';
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
const SOURCES: { name: string; note: string }[] = [
  { name: 'Spark', note: 'Spark check' },
  { name: 'Databricks', note: 'Spark check on the driver' },
  { name: 'Airflow', note: 'StatsD metrics' },
  { name: 'dbt', note: 'not supported yet' },
];

/** Data Observability → Data Jobs: Airflow DAGs and Spark applications with their runs. */
export function DataJobsPage() {
  const { range } = useTime();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const [setup, setSetup] = useState(false);
  const page = useQuery({ queryKey: ['data-jobs', range, query], queryFn: () => searchDataJobs({ range, query }), placeholderData: keepPreviousData });
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
        ) : !page.data.enabled ? (
          <JobsHero onSetup={() => setSetup(true)} />
        ) : (
          <JobsList page={page.data} query={query} onQuery={(q) => setParams(q ? { q } : {}, { replace: true })} />
        )}
      </div>
      {setup && (
        <Modal label="Data Jobs setup" onClose={() => setSetup(false)} width={860} height="auto">
          <div className="dbm-modal">
            <header>
              <h2>Send data job metrics to ObserveCraft</h2>
              <button className="icon-btn" onClick={() => setSetup(false)} title="Close">
                <X size={16} />
              </button>
            </header>
            <SetupOptions options={JOB_SETUP} />
          </div>
        </Modal>
      )}
    </div>
  );
}

function JobsHero({ onSetup }: { onSetup: () => void }) {
  return (
    <section className="dbm-hero">
      <div>
        <span className="dbm-hero-kicker">Data Observability</span>
        <h2>Know when a pipeline fails or runs late, before the dashboards it feeds go stale</h2>
        <p>
          Data Jobs lists your Airflow DAGs and Spark applications with their successful and failed runs, durations and failing tasks, from the metrics the Datadog Agent already collects for them.
        </p>
        <ul className="dbm-sources">
          {SOURCES.map((s) => (
            <li key={s.name}>
              <strong>{s.name}</strong>
              <span className="muted">{s.note}</span>
            </li>
          ))}
        </ul>
        <button className="btn btn-primary" onClick={onSetup}>
          Set up Data Jobs
        </button>
      </div>
      <div className="dbm-hero-preview" aria-hidden="true">
        {(['failed', 'running', 'succeeded', 'succeeded'] as JobStatus[]).map((s, i) => (
          <div key={i} className="dbm-hero-row">
            <span className={`dbm-status dbm-status-${s}`}>{STATUS_LABELS[s]}</span>
            <span className="dbm-hero-bar" style={{ width: `${[70, 45, 85, 30][i]}%` }} />
          </div>
        ))}
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
                    <td>{j.framework === 'airflow' ? 'Airflow DAG' : 'Spark application'}</td>
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
