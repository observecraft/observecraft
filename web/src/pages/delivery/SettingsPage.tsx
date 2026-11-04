import { useEffect } from 'react';
import { useLocation, useSearchParams } from 'react-router-dom';
import { Check } from 'lucide-react';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { Step } from '../integrations/Step';
import { Ext, PROVIDER_LABEL, StatusPill, fmtAgo } from './common';
import { GitlabCiSetup } from './GitlabCiSetup';
import { useRepositories } from './queries';
import '../integrations/integrations.css';
import '../integrations/aws/aws.css';

/** CI providers of step 1: GitLab is pulled through its API, the others send OpenTelemetry traces. */
const PROVIDERS: { id: string; name: string; logo: string }[] = [
  { id: 'gitlab', name: 'GitLab CI', logo: 'gitlab' },
  { id: 'github_actions', name: 'GitHub Actions', logo: 'github' },
  { id: 'jenkins', name: 'Jenkins', logo: 'jenkins' },
  { id: 'circleci', name: 'CircleCI', logo: 'circleci' },
  { id: 'buildkite', name: 'Buildkite', logo: 'buildkite' },
  { id: 'azure_pipelines', name: 'Azure Pipelines', logo: 'tile:azure-devops' },
  { id: 'otel', name: 'Other (OpenTelemetry)', logo: 'opentelemetry' },
];

const RESOURCE_ATTRS: [string, string, boolean][] = [
  ['cicd.pipeline.name', 'Pipeline (workflow) name — routes the trace to CI/CD', true],
  ['cicd.pipeline.run.id', 'Run number or id shown as the pipeline ID', true],
  ['cicd.pipeline.run.url.full', 'Link to the run in your CI provider', false],
  ['cicd.pipeline.run.attempt', 'Retry attempt (1, 2, …)', false],
  ['cicd.provider.name', 'github_actions, gitlab, jenkins, circleci, …', false],
  ['vcs.repository.name', 'Repository, e.g. my-org/checkout', true],
  ['vcs.repository.url.full', 'Repository URL', false],
  ['vcs.ref.head.name', 'Branch', true],
  ['vcs.ref.head.revision', 'Commit SHA (flaky-test detection and DORA)', true],
  ['vcs.revision.author / vcs.revision.message', 'Commit author and message', false],
  ['vcs.revision.committed_at', 'Commit time in epoch ms (DORA lead time)', false],
  ['team', 'Owning team handle', false],
];
const SPAN_ATTRS: [string, string][] = [
  ['root span', 'The pipeline run; cicd.pipeline.result = success | failure | cancellation | skip'],
  ['cicd.pipeline.task.name', 'Marks a job span; cicd.pipeline.task.run.result for its result'],
  ['cicd.pipeline.task.type', 'build | test | deploy'],
  ['ci.stage.name', 'Stage the job belongs to'],
  ['ci.step.name', 'Marks a step span inside a job'],
  ['deployment.environment.name', 'On deploy jobs: the environment deployed to (DORA)'],
  ['deployment.service.name', 'On deploy jobs: the service deployed (DORA, incident matching)'],
  ['test.suite.name / test.case.name', 'Marks a test case span'],
  ['test.case.result.status', 'pass | fail'],
];

const EXAMPLE = `curl -X POST http://<collector>:4318/v1/traces -H 'Content-Type: application/json' -d '{
  "resourceSpans": [{
    "resource": { "attributes": [
      { "key": "cicd.pipeline.name",      "value": { "stringValue": "ci" } },
      { "key": "cicd.pipeline.run.id",    "value": { "stringValue": "1842" } },
      { "key": "vcs.repository.name",     "value": { "stringValue": "my-org/checkout" } },
      { "key": "vcs.ref.head.name",       "value": { "stringValue": "main" } },
      { "key": "vcs.ref.head.revision",   "value": { "stringValue": "9fceb02…" } }
    ] },
    "scopeSpans": [{ "spans": [{
      "traceId": "5b8efff798038103d269b633813fc60c", "spanId": "eee19b7ec3c1b174",
      "name": "ci", "kind": 1,
      "startTimeUnixNano": "1730000000000000000", "endTimeUnixNano": "1730000300000000000",
      "attributes": [{ "key": "cicd.pipeline.result", "value": { "stringValue": "success" } }]
    }] }]
  }]
}'`;

/** How to send pipeline data, and the repositories reporting it. */
export function SettingsPage() {
  const repos = useRepositories();
  const { hash } = useLocation();
  const [params, setParams] = useSearchParams();
  const provider = params.get('provider') ?? 'gitlab';
  const setProvider = (id: string) => setParams((p) => new URLSearchParams({ ...Object.fromEntries(p), provider: id }), { replace: true });
  useEffect(() => {
    if (hash) document.getElementById(hash.slice(1))?.scrollIntoView({ behavior: 'smooth' });
  }, [hash]);
  return (
    <div className="ci-settings">
      <section className="ci-card">
        <h3>Repositories</h3>
        {repos.isPending ? (
          <Loading />
        ) : repos.isError ? (
          <ErrorState error={repos.error} onRetry={() => repos.refetch()} />
        ) : repos.data.length === 0 ? (
          <p className="muted">No repository has reported a pipeline in the last 30 days.</p>
        ) : (
          <table className="ci-table">
            <thead>
              <tr>
                <th>REPOSITORY</th>
                <th>PROVIDER</th>
                <th>PIPELINES</th>
                <th className="num">RUNS (30D)</th>
                <th className="num">FAILURES</th>
                <th>LAST RUN</th>
              </tr>
            </thead>
            <tbody>
              {repos.data.map((r) => (
                <tr key={r.repository}>
                  <td>
                    <Ext href={r.url}>{r.repository}</Ext>
                  </td>
                  <td>{PROVIDER_LABEL[r.provider] ?? r.provider}</td>
                  <td>{r.pipelines.join(', ')}</td>
                  <td className="num">{r.runs.toLocaleString()}</td>
                  <td className="num">{r.failures.toLocaleString()}</td>
                  <td>
                    <StatusPill status={r.lastStatus} /> <span className="muted">{fmtAgo(r.lastRun)}</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <section className="ci-card ci-setup" id="add-repository">
        <h3>Add a repository</h3>
        <Step n={1} title="Choose your CI provider" open onToggle={() => {}} done>
          <div className="rt-sources">
            {PROVIDERS.map((p) => (
              <button key={p.id} type="button" className={`rt-source${provider === p.id ? ' selected' : ''}`} onClick={() => setProvider(p.id)}>
                <IntegrationLogo logo={p.logo} name={p.name} size={28} />
                <span>
                  {provider === p.id && <Check size={14} />} {p.name}
                </span>
              </button>
            ))}
          </div>
        </Step>
        {provider === 'gitlab' ? (
          <GitlabCiSetup />
        ) : (
          <Step n={2} title="Send pipelines as OpenTelemetry traces" open onToggle={() => {}}>
            <OtlpInstructions />
          </Step>
        )}
      </section>
    </div>
  );
}

/** How to send pipelines from any CI provider as OpenTelemetry traces. */
function OtlpInstructions() {
  return (
    <div className="ci-otlp">
      <p>
        ObserveCraft reads CI/CD runs as <b>OpenTelemetry traces</b>: one trace per pipeline run, with a span for the pipeline, one per job, and optionally steps and test cases. Send them to the
        OpenTelemetry Collector's OTLP endpoint (port 4317 gRPC / 4318 HTTP) from your CI provider's OpenTelemetry integration or exporter. Any trace whose resource has <code>cicd.pipeline.name</code>{' '}
        is stored as CI data (kept 30 days) instead of APM.
      </p>
      <h4>Resource attributes (per run)</h4>
      <table className="ci-table ci-attr-table">
        <tbody>
          {RESOURCE_ATTRS.map(([k, v, req]) => (
            <tr key={k}>
              <td>
                <code>{k}</code>
                {req && <span className="ci-req">required</span>}
              </td>
              <td>{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h4>Span attributes</h4>
      <table className="ci-table ci-attr-table">
        <tbody>
          {SPAN_ATTRS.map(([k, v]) => (
            <tr key={k}>
              <td>
                <code>{k}</code>
              </td>
              <td>{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h4>Minimal example</h4>
      <pre className="ci-code">{EXAMPLE}</pre>
      <p className="muted">Pipeline runs appear in the Explorer within seconds. Flaky-test detection needs test case spans; DORA metrics need deploy jobs with deployment.environment.name.</p>
    </div>
  );
}
