// In-browser remediation for mock mode (routes/remediation.rs): levels per finding, explanation and fix runs that
// play like the server's Assist runs, deterministic manifest bumps, and merge requests. The demo's GitHub connection
// can write; its GitLab connection is read-only (the diff is copied instead); other hosts have no connection.
import type { CodeChange, RunSummary } from '../assist/types';
import type { FindingRef, FixTarget, FixView, Remediation } from '../remediation/client';
import { addScriptedRun, getRun } from './assistRuns';
import { findingById, libraryByKey } from './codeSecurity';

const ME = 'Alex Morgan';
const TAGGING_POLICIES = '/organization-settings/tagging-policies';
const GITLAB_READ_ONLY = 'The GitLab token lacks the `api` scope, which opening a merge request needs (it has: read_api, read_repository).';

interface MockFix {
  view: Omit<FixView, 'status' | 'change' | 'explanation'>;
  change?: CodeChange;
  explanation?: string;
}

const fixes: MockFix[] = [];
const explanations = new Map<string, string>();
const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `id-${Math.random().toString(36).slice(2)}-${Date.now()}`);
const refKey = (r: FindingRef) => `${r.kind}:${r.id}`;

function connectionFor(repository: string): { blocked: string | null } | string {
  const host = repository.split('/')[0];
  if (host === 'github.com') return { blocked: null };
  if (host === 'gitlab.com') return { blocked: GITLAB_READ_ONLY };
  return `No GitHub or GitLab connection reaches ${host}: add one in Developer Portal → Settings. Read access lets Assist propose a fix; opening a merge request also needs write access.`;
}

const MANIFEST: Record<string, string> = { 'package-lock.json': 'package.json', 'yarn.lock': 'package.json', 'go.sum': 'go.mod', 'poetry.lock': 'pyproject.toml', 'Gemfile.lock': 'Gemfile' };
const manifestOf = (file: string) => {
  const name = file.split('/').pop() ?? file;
  const dir = file.slice(0, file.length - name.length);
  return MANIFEST[name] ? dir + MANIFEST[name] : file;
};

interface Level {
  level: Remediation['level'];
  reason: string;
  targets: FixTarget[];
}

function levelOf(r: FindingRef): Level {
  if (r.kind === 'cloud_misconfiguration' || r.kind === 'cloud_vulnerability')
    return {
      level: 'needs_provenance',
      reason:
        'This finding names a cloud resource, not the code that creates it. Assist can explain it; a fix in code needs the resource’s provenance (tags or labels naming its repository and file), which Tagging Policies define.',
      targets: [],
    };
  if (r.kind === 'library') {
    const v = libraryByKey(r.id);
    if (!v) throw new Error('No such finding');
    const declared = v.occurrences.filter((o) => o.source !== 'runtime' && o.repository);
    if (!declared.length)
      return {
        level: 'needs_provenance',
        reason:
          'Only running services load this library: no repository is known to declare it. With git metadata on the services (DD_GIT_REPOSITORY_URL, or a repository upload of its dependencies) it can be fixed in code; Tagging Policies list what’s missing.',
        targets: [],
      };
    if (!v.fixed) return { level: 'explain', reason: 'No fixed version is published for this advisory yet, so there is nothing to upgrade to.', targets: [] };
    const targets = [...new Map(declared.map((o) => [o.repository, { repository: o.repository, files: [manifestOf(o.file)] }])).values()];
    return { level: 'fix_in_code', reason: `The manifest can be bumped to the first fixed version, ${v.fixed}, without running the package manager.`, targets };
  }
  const f = findingById(r.id);
  if (!f) throw new Error('No such finding');
  const why =
    f.kind === 'secret'
      ? 'Assist proposes a change to the file that removes the secret from the code; the credential must be rotated as well.'
      : f.kind === 'iac'
        ? 'Assist proposes a change to the file that fixes the misconfiguration.'
        : 'Assist proposes a change to the file that fixes the vulnerability.';
  return { level: 'fix_in_code', reason: why, targets: [{ repository: f.repository, files: [f.file] }] };
}

async function view(f: MockFix): Promise<FixView> {
  if (!f.view.runId) return { ...f.view, status: 'ready', change: f.change ?? null, explanation: f.explanation ?? null };
  const run = await getRun(f.view.runId);
  return { ...f.view, status: run.status, change: run.change ?? null, explanation: run.change ? null : (run.conclusion?.summary ?? null) };
}

// ---------------------------------------------------------------- proposals

function manifestChange(pkg: string, version: string, fixed: string, file: string): CodeChange {
  const name = file.split('/').pop() ?? file;
  const [before, after] = ((): [string, string] => {
    const t = (v: string) => {
      if (name === 'package.json') return `{\n  "name": "api-gateway",\n  "private": true,\n  "dependencies": {\n    "${pkg}": "^${v}",\n    "@example/metrics": "^2.4.0"\n  }\n}\n`;
      if (name === 'pom.xml') {
        const [g, a] = pkg.split(':');
        return `<project>\n  <dependencies>\n    <dependency>\n      <groupId>${g}</groupId>\n      <artifactId>${a}</artifactId>\n      <version>${v}</version>\n    </dependency>\n  </dependencies>\n</project>\n`;
      }
      if (name === 'go.mod') return `module github.com/example-org/order-service\n\ngo 1.22\n\nrequire (\n\t${pkg} v${v}\n)\n`;
      if (name.endsWith('.csproj')) return `<Project Sdk="Microsoft.NET.Sdk">\n  <ItemGroup>\n    <PackageReference Include="${pkg}" Version="${v}" />\n  </ItemGroup>\n</Project>\n`;
      return `# billing\n${pkg}==${v}\nexample-http==2.31.0\n`;
    };
    return [t(version), t(fixed)];
  })();
  return {
    title: `Bump ${pkg} to ${fixed}`,
    description: `Bumps \`${pkg}\` from ${version} to ${fixed} in \`${file}\`, the first version that fixes the advisory.`,
    files: [{ path: file, before, after }],
  };
}

const LOCK_NOTE: Record<string, string> = {
  'package.json': 'Regenerate the lockfile (package-lock.json, yarn.lock or pnpm-lock.yaml) with the package manager, in CI or locally (`npm install`): it isn’t edited here.',
  'go.mod': 'Update go.sum (`go mod tidy`): its checksums can’t be computed without downloading the module.',
};

/** Static findings' fixes: the line the finding points at, and what replaces it. */
const CODE_FIXES: Record<string, [string, string]> = {
  'f-sql': [
    'admin_query = f"SELECT * FROM coupons WHERE {custom_filter}"\ncursor.execute(admin_query)',
    'admin_query = "SELECT * FROM coupons WHERE code = %s"\ncursor.execute(admin_query, (custom_filter,))',
  ],
  'f-path': [
    'byte[] body = Files.readAllBytes(Paths.get(baseDir, name));',
    'Path file = Paths.get(baseDir).resolve(name).normalize();\n        if (!file.startsWith(Paths.get(baseDir))) throw new ResponseStatusException(HttpStatus.BAD_REQUEST);\n        byte[] body = Files.readAllBytes(file);',
  ],
  'f-pst': ['e.printStackTrace();', 'log.error("Updating the cart failed", e);'],
  'f-secret': ['PAYMENTS_KEY=pk_live_EXAMPLE…', 'PAYMENTS_KEY=<your payments API key, from the secret store>'],
  'f-iac1': ['resource "aws_dynamodb_table" "orders" {\n  name = "orders"', 'resource "aws_dynamodb_table" "orders" {\n  name = "orders"\n\n  point_in_time_recovery {\n    enabled = true\n  }'],
  'f-iac2': ['  acl    = "public-read"', '  acl    = "private"'],
  'gl-7001': ['  res.send(`<h1>Receipt ${req.query.order}</h1>`);', '  res.send(`<h1>Receipt ${escapeHtml(String(req.query.order))}</h1>`);'],
  'gl-7002': ['  acl    = "public-read-write"', '  acl    = "private"'],
  'gl-7003': ['AWS_ACCESS_KEY_ID=AKIAEXAMPLEEXAMPLE00', 'AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID}'],
};

function codeChange(id: string, file: string, ruleName: string, kind: string): CodeChange {
  const [bad, good] = CODE_FIXES[id] ?? ['// flagged line', '// fixed line'];
  const wrap = (body: string) => `// ${file}\n// (excerpt around the finding)\n\n${body}\n\n// end of excerpt\n`;
  const secret = kind === 'secret' ? ' The credential must be rotated: removing it from the file doesn’t revoke it, and it stays in the history.' : '';
  return { title: `Fix ${ruleName.toLowerCase()} in ${file.split('/').pop()}`, description: `Changes only \`${file}\`.${secret}`, files: [{ path: file, before: wrap(bad), after: wrap(good) }] };
}

function seed() {
  if (fixes.length) return;
  for (const [id, mr] of [
    ['f-path', null],
    ['f-iac2', { url: 'https://github.com/example-org/storefront/pull/4187', number: 4187, branch: 'assist/fix-s3-bucket-is-publicly-readable-3f2a1c0d', draft: true, state: 'open' }],
  ] as const) {
    const f = findingById(id);
    if (!f) continue;
    fixes.push({
      view: {
        id: uuid(),
        findingKind: 'code',
        findingId: id,
        repository: f.repository,
        method: 'assist',
        runId: null,
        notes: [],
        blocked: null,
        mergeRequest: mr,
        createdBy: ME,
        createdAt: Date.now() - 2 * 3_600_000,
      },
      change: codeChange(id, f.file, f.ruleName, f.kind),
    });
  }
}

// ---------------------------------------------------------------- API

export async function remediationOf(r: FindingRef): Promise<Remediation> {
  seed();
  const l = levelOf(r);
  const conn = l.targets[0] ? connectionFor(l.targets[0].repository) : null;
  const explain = explanations.get(refKey(r));
  let explainRun: RunSummary | null = null;
  if (explain) {
    const { steps: _s, conclusion: _c, change: _ch, pullRequest: _p, ...summary } = await getRun(explain);
    explainRun = summary;
  }
  const latest = [...fixes].reverse().find((f) => f.view.findingKind === r.kind && f.view.findingId === r.id);
  return {
    ...l,
    connection: typeof conn === 'string' ? conn : null,
    taggingPolicies: l.level === 'needs_provenance' ? TAGGING_POLICIES : null,
    explainRun,
    fix: latest ? await view(latest) : null,
  };
}

export async function explainFinding(r: FindingRef): Promise<RunSummary> {
  const l = levelOf(r);
  const lib = r.kind === 'library' ? libraryByKey(r.id) : undefined;
  const f = r.kind === 'code' ? findingById(r.id) : undefined;
  const title = lib?.title ?? f?.ruleName ?? (r.kind === 'cloud_vulnerability' ? 'Vulnerable package on a host' : 'Cloud resource misconfiguration');
  const summary = lib
    ? `**${lib.package} ${lib.version}** is affected by ${lib.advisoryId} (${lib.cve ?? 'no CVE'}): ${lib.title}. An attacker who reaches the affected code path can exploit it; ${lib.inProduction ? 'a production service loads it' : 'no production service is known to load it'}.`
    : f
      ? `**${f.ruleName}** at \`${f.file}:${f.line}\`: ${f.message}. ${f.kind === 'secret' ? 'Anyone with read access to the repository (or its history) can use this credential.' : 'Untrusted input reaches this code without validation.'}`
      : 'The resource’s configuration doesn’t meet the rule: it exposes more than it needs to. The fix belongs in the infrastructure code that creates the resource.';
  const steps = lib
    ? [`Upgrade ${lib.package} to ${lib.fixed ?? 'a fixed version once published'}`, 'Regenerate the lockfile and run the tests', 'Redeploy the services that load it']
    : f?.kind === 'secret'
      ? ['Rotate the credential now', 'Read it from the environment or the secret store instead of the file', 'Remove it from the repository history if the repository is shared']
      : f
        ? ['Validate or encode the input where it enters', 'Use the framework’s safe API for this operation', 'Add a test that sends a malicious input']
        : ['Find the Terraform or Helm code that creates the resource', 'Change the setting the rule names', 'Tag the resource with its repository and file so the next fix can be proposed in code'];
  const run = addScriptedRun(
    'explain',
    `Explain: ${title}`,
    { finding: { kind: r.kind, id: r.id } },
    [`Reading the ${l.level === 'fix_in_code' ? 'finding and its code' : 'finding'}…`, 'Explaining what it means and how to remediate it.'],
    { summary, verdict: 'explained', confidence: 'high', evidence: [], nextSteps: steps },
  );
  explanations.set(refKey(r), run.id);
  return run;
}

export async function fixFinding(r: FindingRef): Promise<FixView> {
  const l = levelOf(r);
  if (l.level !== 'fix_in_code') throw new Error(l.reason);
  const target = l.targets.find((t) => !r.repository || t.repository === r.repository) ?? l.targets[0];
  const conn = connectionFor(target.repository);
  if (typeof conn === 'string') throw new Error(conn);
  const base = { id: uuid(), findingKind: r.kind, findingId: r.id, repository: target.repository, blocked: conn.blocked, mergeRequest: null, createdBy: ME, createdAt: Date.now() };
  if (r.kind === 'library') {
    const v = libraryByKey(r.id)!;
    const file = target.files[0];
    const notes = [LOCK_NOTE[file.split('/').pop() ?? ''] ?? 'Regenerate the lockfile with the package manager, in CI: it isn’t edited here.'];
    const fix: MockFix = { view: { ...base, method: 'manifest', runId: null, notes }, change: manifestChange(v.package, v.version, v.fixed!, file) };
    fixes.push(fix);
    return view(fix);
  }
  const f = findingById(r.id)!;
  const change = codeChange(f.id, f.file, f.ruleName, f.kind);
  const run = addScriptedRun(
    'fix',
    `Fix ${f.ruleName} in ${f.file}`,
    { finding: { kind: r.kind, id: r.id }, repository: f.repository, file: f.file, line: f.line },
    [
      `Fixing ${f.ruleName} in ${f.file}.`,
      ['list_nearby_files', {}, '6 files'],
      ['read_repo_file', { path: f.file }, `${f.file}, 184 lines`],
      ['propose_change', { title: change.title }, 'Proposed. Now conclude.'],
    ],
    {
      summary: `${f.message}. The change keeps the behaviour and removes the flaw at line ${f.line}.`,
      verdict: 'change_proposed',
      confidence: 'high',
      evidence: [],
      nextSteps: ['Review the diff', 'Open the merge request and let CI run'],
    },
    change,
  );
  const fix: MockFix = {
    view: {
      ...base,
      method: 'assist',
      runId: run.id,
      notes: f.kind === 'secret' ? ['Rotate the credential: removing it from the code doesn’t revoke it, and it stays in the repository’s history.'] : [],
    },
  };
  fixes.push(fix);
  return view(fix);
}

const find = (id: string) => {
  const f = fixes.find((x) => x.view.id === id);
  if (!f) throw new Error('No such fix');
  return f;
};

export async function getFix(id: string): Promise<FixView> {
  return view(find(id));
}

export async function openMergeRequest(id: string): Promise<FixView> {
  const f = find(id);
  if (f.view.mergeRequest) throw new Error('A merge request was already opened for this fix');
  if (f.view.blocked) throw new Error(f.view.blocked);
  const v = await view(f);
  if (!v.change) throw new Error('This fix proposed no change');
  const number = 4200 + fixes.indexOf(f);
  const slug = v.change.title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .slice(0, 48);
  const [, owner, name] = f.view.repository.split('/');
  f.view.mergeRequest = { url: `https://github.com/${owner}/${name}/pull/${number}`, number, branch: `assist/${slug}-${id.slice(0, 8)}`, draft: true, state: 'open' };
  return view(f);
}
