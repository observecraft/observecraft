import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { Boxes, Check, ChevronDown, Cloud, FunctionSquare, Network, Plus, Server } from 'lucide-react';
import { searchHosts } from '../../api/infrastructure/client';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { CodeBlock } from '../../components/code/CodeBlock';
import { useTime } from '../../components/shell/TimeContext';

type Kind = 'containers' | 'hosts' | 'serverless' | 'aws' | 'azure' | 'google';

interface Env {
  id: string;
  label: string;
  logo: string;
  kinds: Kind[];
  /** Single Step Instrumentation: the Agent injects the tracer into processes it starts. */
  ssi?: boolean;
  os?: 'linux' | 'windows';
}

const ENVS: Env[] = [
  { id: 'ecs-ec2', label: 'Amazon ECS on EC2', logo: 'amazonecs', kinds: ['containers', 'aws'] },
  { id: 'ecs-fargate', label: 'Amazon ECS on Fargate', logo: 'awsfargate', kinds: ['containers', 'serverless', 'aws'] },
  { id: 'lambda', label: 'AWS Lambda Functions', logo: 'awslambda', kinds: ['serverless', 'aws'] },
  { id: 'azure-container-apps', label: 'Azure Container Apps', logo: 'azure', kinds: ['containers', 'serverless', 'azure'] },
  { id: 'azure-functions', label: 'Azure Functions', logo: 'azure', kinds: ['serverless', 'azure'] },
  { id: 'azure-web-apps', label: 'Azure Web Apps', logo: 'azure', kinds: ['serverless', 'azure'] },
  { id: 'docker', label: 'Docker', logo: 'docker', kinds: ['containers'], ssi: true },
  { id: 'cloud-run', label: 'Google Cloud Run', logo: 'googlecloudrun', kinds: ['containers', 'serverless', 'google'] },
  { id: 'cloud-run-functions', label: 'Google Cloud Run Functions', logo: 'googlecloudfunctions', kinds: ['serverless', 'google'] },
  { id: 'cloud-run-jobs', label: 'Google Cloud Run Jobs', logo: 'googlecloudrun', kinds: ['containers', 'serverless', 'google'] },
  { id: 'kubernetes', label: 'Kubernetes', logo: 'kubernetes', kinds: ['containers', 'aws', 'azure', 'google'], ssi: true },
  { id: 'linux', label: 'Linux', logo: 'linux', kinds: ['hosts'], ssi: true, os: 'linux' },
  { id: 'windows', label: 'Windows', logo: 'windows', kinds: ['hosts'], ssi: true, os: 'windows' },
];

const KINDS: { id: Kind; label: string; icon: React.ReactNode }[] = [
  { id: 'containers', label: 'Containers', icon: <Boxes size={15} /> },
  { id: 'hosts', label: 'Hosts', icon: <Server size={15} /> },
  { id: 'serverless', label: 'Serverless', icon: <FunctionSquare size={15} /> },
  { id: 'aws', label: 'AWS', icon: <BrandLogo name="aws" size={15} /> },
  { id: 'azure', label: 'Azure', icon: <BrandLogo name="azure" size={15} /> },
  { id: 'google', label: 'Google', icon: <BrandLogo name="googlecloud" size={15} /> },
];

const LANGS: { label: string; logo: string }[] = [
  { label: 'Java', logo: 'openjdk' },
  { label: 'Python', logo: 'python' },
  { label: 'Ruby', logo: 'ruby' },
  { label: '.NET', logo: 'dotnet' },
  { label: 'Node.js', logo: 'nodedotjs' },
  { label: 'PHP', logo: 'php' },
];

const INTAKE = `${window.location.protocol}//${window.location.hostname}:4318`;

/** Agent configuration per environment: Datadog's own Agent and tracers, pointed at the ObserveCraft intake. */
function setup(env: Env, k8s: 'operator' | 'helm', ssi: boolean): { title: string; code: string; filename: string }[] {
  const tags = `DD_SERVICE=<service>\nDD_ENV=<env>\nDD_VERSION=<version>`;
  switch (env.id) {
    case 'kubernetes':
      return k8s === 'operator'
        ? [
            {
              title: 'Install the Datadog Operator',
              filename: 'shell',
              code: 'helm repo add datadog https://helm.datadoghq.com\nhelm install datadog-operator datadog/datadog-operator -n datadog --create-namespace\nkubectl -n datadog create secret generic datadog-secret --from-literal api-key=<ingest key>',
            },
            {
              title: 'Apply a DatadogAgent that sends to ObserveCraft (the full example is deploy/examples/datadog/datadog-agent.yaml)',
              filename: 'datadog-agent.yaml',
              code: `apiVersion: datadoghq.com/v2alpha1
kind: DatadogAgent
metadata:
  name: datadog
  namespace: datadog
spec:
  global:
    clusterName: <cluster>
    site: observecraft.invalid
    endpoint:
      url: ${INTAKE}
    credentials:
      apiSecret: { secretName: datadog-secret, keyName: api-key }
  features:
    apm:
      enabled: true${ssi ? '\n      instrumentation:\n        enabled: true\n        targets:\n          - name: all-services\n            ddTraceVersions: { java: "1", python: "3", js: "5", dotnet: "3", ruby: "2", php: "1" }' : ''}
  override:
    nodeAgent:
      env:
        - { name: DD_APM_DD_URL, value: ${INTAKE} }
        - { name: DD_APM_PROFILING_DD_URL, value: ${INTAKE}/api/v2/profile }`,
            },
          ]
        : [
            {
              title: 'Add the Datadog chart',
              filename: 'shell',
              code: 'helm repo add datadog https://helm.datadoghq.com\nkubectl -n datadog create secret generic datadog-secret --from-literal api-key=<ingest key>',
            },
            {
              title: 'Install it with values that send to ObserveCraft (the full example is deploy/examples/datadog/values.yaml)',
              filename: 'values.yaml',
              code: `datadog:
  apiKeyExistingSecret: datadog-secret
  clusterName: <cluster>
  site: observecraft.invalid
  dd_url: ${INTAKE}
  apm:
    portEnabled: true${ssi ? '\n    instrumentation:\n      enabled: true\n      targets:\n        - name: all-services\n          ddTraceVersions: { java: "1", python: "3", js: "5", dotnet: "3", ruby: "2", php: "1" }' : ''}
  env:
    - { name: DD_APM_DD_URL, value: ${INTAKE} }
    - { name: DD_APM_PROFILING_DD_URL, value: ${INTAKE}/api/v2/profile }
clusterAgent:
  enabled: true`,
            },
            { title: 'Install', filename: 'shell', code: 'helm install datadog datadog/datadog -n datadog -f values.yaml' },
          ];
    case 'linux':
      return [
        {
          title: ssi ? 'Install the Agent with Single Step Instrumentation' : 'Install the Agent',
          filename: 'shell',
          code: `DD_API_KEY=<ingest key> DD_SITE=observecraft.invalid ${ssi ? 'DD_APM_INSTRUMENTATION_ENABLED=host DD_APM_INSTRUMENTATION_LIBRARIES="java:1,python:3,js:5,dotnet:3,ruby:2,php:1" ' : ''}\\\n  bash -c "$(curl -L https://install.datadoghq.com/scripts/install_script_agent7.sh)"`,
        },
        {
          title: 'Point it at ObserveCraft in /etc/datadog-agent/datadog.yaml, then restart the Agent',
          filename: 'datadog.yaml',
          code: `dd_url: ${INTAKE}\napm_config:\n  enabled: true\n  apm_dd_url: ${INTAKE}`,
        },
      ];
    case 'windows':
      return [
        {
          title: 'Install the Agent (PowerShell, as administrator)',
          filename: 'powershell',
          code: 'Start-Process msiexec -Wait -ArgumentList \'/qn /i datadog-agent-7-latest.amd64.msi APIKEY="<ingest key>" SITE="observecraft.invalid"\'',
        },
        {
          title: 'Point it at ObserveCraft in C:\\ProgramData\\Datadog\\datadog.yaml, then restart the Agent',
          filename: 'datadog.yaml',
          code: `dd_url: ${INTAKE}\napm_config:\n  enabled: true\n  apm_dd_url: ${INTAKE}`,
        },
      ];
    case 'docker':
      return [
        {
          title: 'Run the Agent next to your containers',
          filename: 'shell',
          code: `docker run -d --name datadog-agent \\
  -e DD_API_KEY=<ingest key> -e DD_SITE=observecraft.invalid \\
  -e DD_DD_URL=${INTAKE} -e DD_APM_DD_URL=${INTAKE} \\
  -e DD_APM_ENABLED=true -e DD_APM_NON_LOCAL_TRAFFIC=true${ssi ? ' \\\n  -e DD_APM_INSTRUMENTATION_ENABLED=docker' : ''} \\
  -v /var/run/docker.sock:/var/run/docker.sock:ro -v /proc/:/host/proc/:ro -v /sys/fs/cgroup/:/host/sys/fs/cgroup:ro \\
  -p 8126:8126 gcr.io/datadoghq/agent:7`,
        },
        { title: 'Tag each service container', filename: 'env', code: `${tags}\nDD_AGENT_HOST=datadog-agent` },
      ];
    default:
      return [
        {
          title: `Add the Datadog tracer to your ${env.label} service and send its traces through an Agent (or the serverless extension) configured for ObserveCraft`,
          filename: 'env',
          code: `DD_API_KEY=<ingest key>\nDD_SITE=observecraft.invalid\nDD_DD_URL=${INTAKE}\nDD_APM_DD_URL=${INTAKE}\n${tags}`,
        },
      ];
  }
}

/** Hosts reporting through the Agent without traces, per OS. */
function useHostsWithoutApm() {
  const { range } = useTime();
  return useQuery({
    queryKey: ['apm', 'instrument', 'hosts', range.from],
    queryFn: () => searchHosts({ range, query: '', metricFilters: [], groupBy: null, sort: { field: 'name', dir: 'asc' }, offset: 0, limit: 2000 }),
    select: (page) => {
      const out = { linux: 0, windows: 0 };
      for (const h of page.hosts) if (h.errorSpans == null && (h.os === 'linux' || h.os === 'windows')) out[h.os] += 1;
      return out;
    },
  });
}

/** APM → Instrument a Service: pick where the services run, get the Agent setup that traces them. */
export function InstrumentPage() {
  const [kind, setKind] = useState<Kind | null>(null);
  const [env, setEnv] = useState<Env | null>(null);
  const [ssi, setSsi] = useState(true);
  const [k8s, setK8s] = useState<'operator' | 'helm'>('operator');
  const [ready, setReady] = useState(false);
  const [open, setOpen] = useState(true);
  const hosts = useHostsWithoutApm();
  const counts = useMemo(() => Object.fromEntries(KINDS.map((k) => [k.id, ENVS.filter((e) => e.kinds.includes(k.id)).length])) as Record<Kind, number>, []);
  const shown = ENVS.filter((e) => !kind || e.kinds.includes(kind));
  const pick = (e: Env | null) => {
    setEnv(e);
    setReady(false);
    setSsi(!!e?.ssi);
  };
  return (
    <div className="apm-instrument">
      <aside className="apm-instrument-nav">
        <Link to="/apm/instrument" className="active">
          <Plus size={15} />
          <span>
            Instrument a Service
            <small>Guided onboarding</small>
          </span>
        </Link>
      </aside>
      <div className="apm-instrument-main">
        <h1>
          <span className="apm-instrument-icon">
            <Network size={20} />
          </span>
          Instrument a Service
        </h1>
        <section className="apm-instrument-step">
          <button className="apm-instrument-step-head" onClick={() => setOpen((o) => !o)}>
            <span className="apm-instrument-num">1</span>
            Where are your services deployed?
            <ChevronDown size={18} className={open ? '' : 'rot'} />
          </button>
          {open &&
            (env ? (
              <div className="apm-instrument-body">
                <div className="apm-instrument-selected">
                  <b>Selected environment</b>
                  <button className="link-btn" onClick={() => pick(null)}>
                    Change Environment
                  </button>
                </div>
                <div className="apm-instrument-card selected">
                  <BrandLogo name={env.logo} size={20} /> {env.label}
                </div>
                {env.ssi && (
                  <div className="apm-instrument-ssi">
                    <div>
                      <Check size={16} />
                      <span>
                        <b>{ssi ? 'Single Step Instrumentation:' : 'Manual instrumentation:'}</b>{' '}
                        {ssi ? 'the Agent adds the tracing library to the services it starts, without code changes.' : 'add the Datadog tracing library to each service yourself.'}
                      </span>
                      <button className="link-btn" title={`Switch to ${ssi ? 'manual instrumentation' : 'Single Step Instrumentation'}`} onClick={() => (setSsi((s) => !s), setReady(false))}>
                        Switch
                      </button>
                    </div>
                    <div className="apm-instrument-langs">
                      <span className="muted">Supported languages</span>
                      {LANGS.map((l) => (
                        <span key={l.label} className="apm-instrument-lang">
                          <BrandLogo name={l.logo} size={14} /> {l.label}
                        </span>
                      ))}
                    </div>
                  </div>
                )}
                {env.id === 'kubernetes' && (
                  <>
                    <b className="apm-instrument-q">How do you manage your Datadog Cluster Agents?</b>
                    <div className="apm-instrument-choices">
                      {(
                        [
                          ['operator', 'Datadog Operator', 'kubernetes'],
                          ['helm', 'Helm Chart', 'helm'],
                        ] as const
                      ).map(([id, label, logo]) => (
                        <button key={id} className={`apm-instrument-card${k8s === id ? ' selected' : ''}`} onClick={() => (setK8s(id), setReady(false))}>
                          <BrandLogo name={logo} size={18} /> {label}
                        </button>
                      ))}
                    </div>
                  </>
                )}
                {!ready ? (
                  <button className="btn btn-primary apm-instrument-continue" onClick={() => setReady(true)}>
                    Continue
                  </button>
                ) : (
                  <ol className="apm-instrument-steps">
                    {setup(env, k8s, ssi && !!env.ssi).map((s) => (
                      <li key={s.title}>
                        <p>{s.title}</p>
                        <CodeBlock code={s.code} filename={s.filename} />
                      </li>
                    ))}
                    <li>
                      <p>
                        Set <code>DD_SERVICE</code>, <code>DD_ENV</code> and <code>DD_VERSION</code> on each service and send it a few requests: it shows up on the <Link to="/apm">APM home</Link>{' '}
                        within a minute. Create the ingest key in <Link to="/organization-settings/api-keys">API Keys</Link>.
                      </p>
                    </li>
                  </ol>
                )}
              </div>
            ) : (
              <>
                <div className="apm-instrument-filter">
                  <span>Filter by deployment type</span>
                  <div>
                    {KINDS.map((k) => (
                      <button key={k.id} className={`apm-instrument-chip${kind === k.id ? ' active' : ''}`} onClick={() => setKind((c) => (c === k.id ? null : k.id))}>
                        {k.icon} {k.label} <span className="apm-instrument-count">{counts[k.id]}</span>
                      </button>
                    ))}
                  </div>
                </div>
                <div className="apm-instrument-body">
                  <span className="muted">{kind ? KINDS.find((k) => k.id === kind)!.label : 'All environments'}</span>
                  <div className="apm-instrument-grid">
                    {shown.map((e) => {
                      const n = e.os ? hosts.data?.[e.os] : undefined;
                      return (
                        <button key={e.id} className="apm-instrument-card" onClick={() => pick(e)}>
                          <BrandLogo name={e.logo} size={22} />
                          <span>
                            {e.label}
                            {!!n && (
                              <small>
                                <b>
                                  {n} host{n === 1 ? '' : 's'}
                                </b>{' '}
                                without APM
                              </small>
                            )}
                          </span>
                        </button>
                      );
                    })}
                    {shown.length === 0 && (
                      <span className="muted">
                        <Cloud size={14} /> No environment of this type.
                      </span>
                    )}
                  </div>
                </div>
              </>
            ))}
        </section>
      </div>
    </div>
  );
}
