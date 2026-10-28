// Pure logic of Deployment Gates and PR Gates: monitor matching and verdicts (as the server does
// them, for the mock), the CI/CD snippets that call the evaluation API, suggested PR rules and
// evaluation filters. No React here, so node tests import it directly.
import type { GateEvaluationStatus, PrGateRuleInput, PrGateSource } from '../../api/deliveryGates/client';

/** Whether a monitor matches a monitor rule's query: `key:value` words are tags (`key:*` = any value), other words are in the name. */
export function monitorMatches(query: string, name: string, tags: string[]): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return false;
  const lname = name.toLowerCase();
  const ltags = tags.map((t) => t.toLowerCase());
  return words.every((w) => {
    const i = w.indexOf(':');
    if (i < 0) return lname.includes(w);
    if (w.slice(i + 1) === '*') return ltags.some((t) => t.split(':')[0] === w.slice(0, i));
    return ltags.includes(w);
  });
}

/** Failed when a rule outside dry run failed (and the gate isn't in dry run); dry run when only dry-run rules failed. */
export function gateVerdict(gateDryRun: boolean, results: { status: 'passed' | 'failed'; dryRun: boolean }[]): GateEvaluationStatus {
  const failed = results.filter((r) => r.status === 'failed');
  if (failed.some((r) => !r.dryRun && !gateDryRun)) return 'failed';
  return failed.length ? 'dry_run' : 'passed';
}

export const STATUS_LABEL: Record<GateEvaluationStatus, string> = { passed: 'Passed', failed: 'Failed', dry_run: 'Dry run' };

export const MONITOR_STATUS_LABEL: Record<string, string> = { ok: 'OK', warn: 'Warn', alert: 'Alert', nodata: 'No Data' };

// ================================================================ CI/CD snippets

export type CiToolId = 'argo' | 'github' | 'gitlab' | 'jenkins' | 'circleci' | 'custom';

/** The tools of the Get Started row; `logo` is a key of IntegrationLogo's registry. */
export const CI_TOOLS: { id: CiToolId; label: string; logo: string; note: string }[] = [
  { id: 'argo', label: 'Argo Rollouts', logo: 'argo', note: 'An AnalysisTemplate whose web metric asks the gate; reference it from a canary step or a pre-promotion analysis.' },
  { id: 'github', label: 'GitHub', logo: 'github', note: 'A GitHub Actions step before the deploy step. Store the application key as the OC_APP_KEY repository secret.' },
  { id: 'gitlab', label: 'GitLab', logo: 'gitlab', note: 'A job in the deploy stage, before the deployment job. Add OC_APP_KEY as a masked CI/CD variable.' },
  { id: 'jenkins', label: 'Jenkins', logo: 'jenkins', note: 'A pipeline stage before the deployment. Keep the application key in a secret-text credential named oc-app-key.' },
  { id: 'circleci', label: 'CircleCI', logo: 'circleci', note: 'A job that runs before the deploy job in your workflow. Put OC_APP_KEY in a context or project environment variable.' },
  { id: 'custom', label: 'Custom', logo: 'tile:custom', note: 'Any script or deployment tool: one HTTP request; `pass` tells whether to go on.' },
];

export interface SnippetOptions {
  /** This install's public URL, e.g. https://observecraft.example.com */
  apiUrl: string;
  service: string;
  env: string;
  identifier: string;
}

const quote = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;

/** The shell lines that evaluate the gate and fail when it doesn't pass. `version` is a shell expression. */
function curlLines(o: SnippetOptions, version: string): string[] {
  return [
    `jq -n --arg service ${quote(o.service)} --arg env ${quote(o.env)} --arg identifier ${quote(o.identifier)} --arg version "${version}" \\`,
    `  '{$service, $env, $identifier, $version}' \\`,
    `  | curl -sS --fail-with-body -X POST "${o.apiUrl}/api/v1/deployment-gates/evaluate" \\`,
    `      -H "Authorization: Bearer $OC_APP_KEY" -H "Content-Type: application/json" -d @- \\`,
    `  | tee gate.json`,
    `jq -e '.pass' gate.json > /dev/null`,
  ];
}

const indent = (lines: string[], n: number) => lines.map((l) => ' '.repeat(n) + l).join('\n');

/** A snippet that calls the evaluation API from `tool` for the gate in `o`. */
export function ciSnippet(tool: CiToolId, o: SnippetOptions): string {
  switch (tool) {
    case 'argo':
      return [
        'apiVersion: argoproj.io/v1alpha1',
        'kind: AnalysisTemplate',
        'metadata:',
        '  name: observecraft-deployment-gate',
        'spec:',
        '  args:',
        '    - name: version',
        '  metrics:',
        '    - name: deployment-gate',
        '      count: 1',
        '      successCondition: result == true',
        '      provider:',
        '        web:',
        '          method: POST',
        `          url: "${o.apiUrl}/api/v1/deployment-gates/evaluate"`,
        '          timeoutSeconds: 120',
        '          headers:',
        '            - key: Content-Type',
        '              value: application/json',
        '            # The secret value is "Bearer <application key>".',
        '            - key: Authorization',
        '              valueFrom:',
        '                secretKeyRef:',
        '                  name: observecraft',
        '                  key: OC_APP_KEY',
        '          jsonBody:',
        `            service: ${JSON.stringify(o.service)}`,
        `            env: ${JSON.stringify(o.env)}`,
        `            identifier: ${JSON.stringify(o.identifier)}`,
        '            version: "{{args.version}}"',
        '          jsonPath: "{$.pass}"',
      ].join('\n');
    case 'github':
      return [
        'steps:',
        '  - name: Deployment gate',
        '    env:',
        '      OC_APP_KEY: ${{ secrets.OC_APP_KEY }}',
        '      # The version your spans report as service.version.',
        '      VERSION: ${{ github.sha }}',
        '    run: |',
        indent(curlLines(o, '$VERSION'), 6),
      ].join('\n');
    case 'gitlab':
      return [
        'deployment-gate:',
        '  stage: deploy',
        '  image: alpine:3.20',
        '  before_script:',
        '    - apk add --no-cache curl jq',
        '  script:',
        '    # OC_APP_KEY is a masked CI/CD variable; the version is what your spans report as service.version.',
        '    - |',
        indent(curlLines(o, '$CI_COMMIT_SHORT_SHA'), 6),
      ].join('\n');
    case 'jenkins':
      return [
        "stage('Deployment gate') {",
        '  steps {',
        "    withCredentials([string(credentialsId: 'oc-app-key', variable: 'OC_APP_KEY')]) {",
        "      sh '''",
        indent(curlLines(o, '$GIT_COMMIT'), 8),
        "      '''",
        '    }',
        '  }',
        '}',
      ].join('\n');
    case 'circleci':
      return [
        'jobs:',
        '  deployment-gate:',
        '    docker:',
        '      - image: cimg/base:current',
        '    steps:',
        '      - run:',
        '          name: Deployment gate',
        '          # OC_APP_KEY comes from a context or the project environment variables.',
        '          command: |',
        indent(curlLines(o, '$CIRCLE_SHA1'), 12),
      ].join('\n');
    case 'custom':
      return ['# OC_APP_KEY: an application key (Organization Settings → API Keys); VERSION: the version being deployed.', ...curlLines(o, '$VERSION')].join('\n');
  }
}

/** The snippet a PR check runs (commit SHA from the CI environment). */
export function prCheckSnippet(apiUrl: string): string {
  return [
    '# OC_APP_KEY: an application key; REPOSITORY: example-org/billing; SHA: the pull request head commit.',
    `jq -n --arg repository "$REPOSITORY" --arg sha "$SHA" '{$repository, $sha}' \\`,
    `  | curl -sS --fail-with-body -X POST "${apiUrl}/api/v1/pr-gates/evaluate" \\`,
    `      -H "Authorization: Bearer $OC_APP_KEY" -H "Content-Type: application/json" -d @- \\`,
    '  | tee pr-gates.json',
    "jq -e '.pass' pr-gates.json > /dev/null",
  ].join('\n');
}

// ================================================================ PR gates

export const PR_SOURCES: Record<PrGateSource, { label: string; description: string; evaluable: boolean; finding: string }> = {
  pipeline_failures: { label: 'CI pipeline failures', description: 'Pipeline runs of the commit that failed', evaluable: true, finding: 'failed pipelines' },
  test_failures: { label: 'Test failures', description: 'Tests that failed on the commit and never passed on it', evaluable: true, finding: 'failed tests' },
  flaky_tests: { label: 'Flaky tests', description: 'Tests that both passed and failed on the commit', evaluable: true, finding: 'flaky tests' },
  code_vulnerabilities: { label: 'Code vulnerabilities', description: 'Vulnerabilities found by static analysis of the changed code', evaluable: false, finding: 'code vulnerabilities' },
  code_quality: { label: 'Code quality', description: 'Code quality violations in the changed code', evaluable: false, finding: 'code quality violations' },
  library_vulnerabilities: {
    label: 'Library vulnerabilities',
    description: 'Known vulnerabilities in the open-source libraries the change adds',
    evaluable: false,
    finding: 'library vulnerabilities',
  },
  license_violations: { label: 'License violations', description: 'Libraries under licenses your organization does not allow', evaluable: false, finding: 'license violations' },
};

export const PR_SOURCE_ORDER: PrGateSource[] = ['test_failures', 'flaky_tests', 'pipeline_failures', 'code_vulnerabilities', 'code_quality', 'library_vulnerabilities', 'license_violations'];

const suggestion = (title: string, description: string, source: PrGateSource, minSeverity: PrGateRuleInput['minSeverity']) => ({
  title,
  description,
  input: { name: title, source, repositories: [], threshold: 0, minSeverity, blocking: true, enabled: true } satisfies PrGateRuleInput,
});

/** The suggested rules of the first-run page; each opens the editor prefilled. */
export const PR_SUGGESTIONS = [
  suggestion('Code Vulnerabilities', 'Stop merges that add high or critical vulnerabilities to the code.', 'code_vulnerabilities', 'high'),
  suggestion('Code Quality', 'Keep code quality violations of medium severity or more out of the main branch.', 'code_quality', 'medium'),
  suggestion('Library Vulnerabilities', 'Block dependencies with known critical vulnerabilities.', 'library_vulnerabilities', 'critical'),
  suggestion('License Violations', 'Catch libraries whose licenses your organization does not accept.', 'license_violations', 'high'),
];

export const NEW_PR_RULE: PrGateRuleInput = { name: '', source: 'test_failures', repositories: [], threshold: 0, minSeverity: 'high', blocking: true, enabled: true };

// ================================================================ evaluations

/** Evaluations whose service, environment, identifier and version contain every word, with the status if given. */
export function filterEvaluations<T extends { service: string; env: string; identifier: string; version: string; status: GateEvaluationStatus }>(
  evals: T[],
  text: string,
  status: GateEvaluationStatus | null,
): T[] {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  return evals.filter((e) => {
    if (status && e.status !== status) return false;
    const hay = `${e.service} ${e.env} ${e.identifier} ${e.version}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}
