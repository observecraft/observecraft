import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { Check, ChevronRight, Code2, ExternalLink, ScanEye, ShieldCheck } from 'lucide-react';
import { setup } from '../../api/codeSecurity/client';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { fmtDate } from './common';

const PROVIDERS = [
  { id: 'github', label: 'GitHub', logo: 'github' },
  { id: 'gitlab', label: 'GitLab', logo: 'gitlab' },
  { id: 'bitbucket', label: 'Bitbucket', logo: 'bitbucket' },
  { id: 'other', label: 'Other', logo: '' },
] as const;

const LANGS = [
  { id: 'java', label: 'Java', logo: 'openjdk' },
  { id: 'nodejs', label: 'Node.js', logo: 'nodedotjs' },
  { id: 'python', label: 'Python', logo: 'python' },
  { id: 'dotnet', label: '.NET', logo: 'dotnet' },
] as const;

/** datadog-ci sends SBOMs to `https://<DD_SITE>/` and SARIF reports to `https://cicodescan-intake.<DD_SITE>/`. */
function ciSnippet(provider: (typeof PROVIDERS)[number]['id'], site: string): { code: string; filename: string } {
  const steps = `# 1. Libraries: any CycloneDX generator (cdxgen, syft, …), then
npx @datadog/datadog-ci sbom upload sbom.cdx.json
# 2. Code, secrets and IaC: any SARIF report (Datadog static analyzer, Semgrep, Gitleaks, Checkov, …)
npx @datadog/datadog-ci sarif upload results.sarif`;
  if (provider === 'github')
    return {
      filename: '.github/workflows/code-security.yml',
      code: `on: [push, pull_request]
jobs:
  code-security:
    runs-on: ubuntu-latest
    env:
      DD_SITE: ${site}
      DD_API_KEY: \${{ secrets.OC_INGEST_KEY }}      # an ObserveCraft ingest key (ocik_…)
      DD_APP_KEY: \${{ secrets.OC_APPLICATION_KEY }} # an ObserveCraft application key (ocak_…), for the SBOM
    steps:
      - uses: actions/checkout@v4
      - run: npx @cyclonedx/cdxgen -o sbom.cdx.json
      - run: npx @datadog/datadog-ci sbom upload sbom.cdx.json
      - run: npx @datadog/datadog-ci sarif upload results.sarif`,
    };
  if (provider === 'gitlab')
    return {
      filename: '.gitlab-ci.yml',
      code: `code-security:
  image: node:22
  variables:
    DD_SITE: ${site}
    # DD_API_KEY (ingest key) and DD_APP_KEY (application key) as masked CI/CD variables
  script:
    - npx @cyclonedx/cdxgen -o sbom.cdx.json
    - npx @datadog/datadog-ci sbom upload sbom.cdx.json
    - npx @datadog/datadog-ci sarif upload results.sarif`,
    };
  return { filename: 'shell', code: `export DD_SITE=${site}\nexport DD_API_KEY=<ingest key> DD_APP_KEY=<application key>\n${steps}` };
}

function Status({ ok, children }: { ok: boolean; children: React.ReactNode }) {
  return (
    <span className={`cs-status${ok ? ' ok' : ''}`}>
      {ok ? <Check size={13} /> : null} {children}
    </span>
  );
}

/** Code Security → Settings: what is set up, and how to send each kind of data. */
export function SettingsPage() {
  const q = useQuery({ queryKey: ['code-security', 'setup'], queryFn: ({ signal }) => setup(signal) });
  const [provider, setProvider] = useState<(typeof PROVIDERS)[number]['id']>('github');
  const [lang, setLang] = useState<(typeof LANGS)[number]['id']>('java');
  const site = window.location.hostname;
  const ci = ciSnippet(provider, site);
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const s = q.data;
  return (
    <div className="cs-settings">
      <aside className="cs-side">
        <div className="cs-side-head">PRODUCTS</div>
        <span className="cs-side-group">Code Security</span>
        <button className="active">Setup</button>
      </aside>
      <div className="cs-settings-main">
        <div className="cs-crumbs">
          Security <ChevronRight size={14} /> <Link to="/code-security">Code Security</Link> <ChevronRight size={14} /> Settings
        </div>
        <span className="muted cs-eyebrow">CODE SECURITY</span>
        <h1>Get started with Code Security</h1>
        <p className="muted">Code Security uses what Datadog's tracers, Agent and datadog-ci already send: point them at ObserveCraft and each product below fills in.</p>

        <section className="cs-setup-card">
          <ShieldCheck size={22} />
          <div>
            <h3>Runtime detection of library vulnerabilities</h3>
            <p className="muted">Runtime Software Composition Analysis: the libraries each service loads, from the tracers' telemetry.</p>
            <Status ok={s.runtimeServices > 0}>{s.runtimeServices} services reporting</Status>
            <CodeBlock
              filename="Datadog Agent (Helm values)"
              code={`datadog:\n  env:\n    - { name: DD_APM_TELEMETRY_ENABLED, value: "true" }\n    - { name: DD_APM_TELEMETRY_DD_URL, value: http://<observecraft-intake>:4318 }`}
            />
          </div>
        </section>

        <section className="cs-setup-card">
          <ScanEye size={22} />
          <div>
            <h3>Runtime detection of code vulnerabilities</h3>
            <p className="muted">Runtime Code Analysis (IAST): the tracers report injection, SSRF, path traversal and weak cryptography where requests exercise them.</p>
            <Status ok={s.iastServices > 0}>{s.iastServices} services with findings in the past 7 days</Status>
            <div className="cs-choices">
              {LANGS.map((l) => (
                <button key={l.id} className={`cs-choice${lang === l.id ? ' active' : ''}`} onClick={() => setLang(l.id)}>
                  <BrandLogo name={l.logo} size={20} /> {l.label}
                </button>
              ))}
            </div>
            <CodeBlock
              filename="service environment"
              code={`DD_IAST_ENABLED=true\nDD_SERVICE=<service>\nDD_ENV=<env>${lang === 'java' ? '\n# with dd-java-agent.jar on the JVM: -javaagent:/path/to/dd-java-agent.jar' : ''}`}
            />
          </div>
        </section>

        <section className="cs-setup-card">
          <Code2 size={22} />
          <div>
            <h3>Scanning of your repositories</h3>
            <p className="muted">Software Composition Analysis (SCA), Static Code Analysis (SAST), Secret Scanning and Infrastructure as Code (IaC), uploaded from CI.</p>
            <Status ok={s.repositoriesWithLibraries + s.repositoriesWithCodeScans > 0}>
              {s.repositoriesWithLibraries} repositories with an SBOM · {s.repositoriesWithCodeScans} with code scans
            </Status>
            <h4>1. Select your source code management provider</h4>
            <div className="cs-choices">
              {PROVIDERS.map((p) => (
                <button key={p.id} className={`cs-choice${provider === p.id ? ' active' : ''}`} onClick={() => setProvider(p.id)}>
                  {p.logo && <BrandLogo name={p.logo} size={20} />} {p.label}
                </button>
              ))}
            </div>
            <h4>2. Run the scans in CI</h4>
            <CodeBlock filename={ci.filename} code={ci.code} />
            <p className="muted cs-note">
              datadog-ci sends SBOMs to <code>https://{site}/</code> and SARIF reports to <code>https://cicodescan-intake.{site}/</code>: route both hosts (the second to the intake). A{' '}
              <code>DD_SITE</code> of two labels gets <code>app.</code> prepended. Scans of pull requests (with <code>DD_PR_NUMBER</code> or the CI's pull request variables) are listed under PR
              Security.
            </p>
          </div>
        </section>

        <section className="cs-setup-card">
          <ExternalLink size={22} />
          <div>
            <h3>Vulnerability database</h3>
            <p className="muted">Libraries are matched against the OSV.dev advisories of the packages in use (OC_OSV_URL; set it to off on instances without internet access).</p>
            <Status ok={s.osvEnabled}>{s.osvEnabled ? `Lookups on · last ${fmtDate(s.lastOsvCheck)}` : 'Lookups off'}</Status> <span className="muted">{s.advisories} advisories stored</span>
          </div>
        </section>
      </div>
    </div>
  );
}
