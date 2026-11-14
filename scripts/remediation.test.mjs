import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});
const { unifiedDiff } = await import('../web/src/lib/unifiedDiff.ts');
const mock = await import('../web/src/api/mock/remediation.ts');
const cs = await import('../web/src/api/mock/codeSecurity.ts');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

test('a proposed change is copied as a unified diff', () => {
  const diff = unifiedDiff({
    title: 't',
    description: '',
    files: [
      { path: 'package.json', before: '{\n  "a": "^1.0.0"\n}\n', after: '{\n  "a": "^1.0.1"\n}\n' },
      { path: 'NEW.md', before: null, after: 'x\n' },
    ],
  });
  assert.equal(diff, '--- a/package.json\n+++ b/package.json\n {\n-  "a": "^1.0.0"\n+  "a": "^1.0.1"\n }\n--- /dev/null\n+++ b/NEW.md\n+x\n');
});

test('each kind of finding gets its level', async () => {
  const libs = cs.libraryVulnerabilities({ status: 'all', scope: 'all', limit: 500 }).vulnerabilities;
  const inRepo = libs.find((v) => v.occurrences.some((o) => o.repository.startsWith('github.com/')));
  const runtimeOnly = libs.find((v) => v.occurrences.every((o) => o.source === 'runtime'));
  assert.equal((await mock.remediationOf({ kind: 'library', id: inRepo.key })).level, 'fix_in_code');
  if (runtimeOnly) assert.equal((await mock.remediationOf({ kind: 'library', id: runtimeOnly.key })).level, 'needs_provenance');
  assert.equal((await mock.remediationOf({ kind: 'code', id: 'f-sql' })).level, 'fix_in_code');
  const cloud = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: 'misconfig:ocs-k8s-001:uid-5-node-debugger' });
  assert.equal(cloud.level, 'needs_provenance');
  assert.equal(cloud.taggingPolicies, '/infrastructure/tagging/policies');
  // Seeded: a proposal waiting for review and one with an open merge request.
  assert.equal((await mock.remediationOf({ kind: 'code', id: 'f-path' })).fix.mergeRequest, null);
  assert.equal((await mock.remediationOf({ kind: 'code', id: 'f-iac2' })).fix.mergeRequest.state, 'open');
});

test('a library fix bumps the manifest and opens a merge request once; a read-only connection copies the diff', async () => {
  const libs = cs.libraryVulnerabilities({ status: 'all', scope: 'all', limit: 500 }).vulnerabilities;
  const gh = libs.find((v) => v.occurrences.some((o) => o.repository.startsWith('github.com/')));
  const fix = await mock.fixFinding({ kind: 'library', id: gh.key, repository: gh.occurrences.find((o) => o.repository.startsWith('github.com/')).repository });
  assert.equal(fix.status, 'ready');
  assert.ok(fix.change.files[0].after.includes(gh.fixed) && fix.change.files[0].before.includes(gh.version));
  const opened = await mock.openMergeRequest(fix.id);
  assert.equal(opened.mergeRequest.state, 'open');
  await assert.rejects(mock.openMergeRequest(fix.id), /already opened/);
  assert.equal((await mock.remediationOf({ kind: 'library', id: gh.key })).fix.mergeRequest.number, opened.mergeRequest.number);

  const gl = libs.find((v) => v.occurrences.some((o) => o.repository.startsWith('gitlab.com/')));
  const blocked = await mock.fixFinding({ kind: 'library', id: gl.key, repository: 'gitlab.com/example-org/checkout' });
  assert.match(blocked.blocked, /`api` scope/);
  await assert.rejects(mock.openMergeRequest(blocked.id), /`api` scope/);
});

test('a code fix and an explanation are Assist runs that conclude', async () => {
  const fix = await mock.fixFinding({ kind: 'code', id: 'f-secret' });
  assert.equal(fix.method, 'assist');
  assert.ok(fix.notes[0].includes('Rotate'));
  const run = await mock.explainFinding({ kind: 'cloud_misconfiguration', id: 'misconfig:ocs-k8s-001:uid-5-node-debugger' });
  await sleep(4500);
  const done = await mock.getFix(fix.id);
  assert.equal(done.status, 'concluded');
  assert.ok(done.change.files[0].after.includes('secret store'));
  const r = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: 'misconfig:ocs-k8s-001:uid-5-node-debugger' });
  assert.equal(r.explainRun.id, run.id);
  assert.equal(r.explainRun.verdict, 'explained');
});

const cloudSec = await import('../web/src/api/mock/cloudSecurity.ts');
const misconfig = async (rule, name) => (await cloudSec.misconfigurations('all')).findings.find((f) => f.ruleId === rule && f.resourceName === name).key;
const vuln = (name, kind = 'container_image') => cloudSec.allVulns().find((v) => v.resourceKind === kind && v.resourceName.includes(name)).key;

test('cloud findings are fixed in infrastructure when their source resolves, else say what is missing', async () => {
  const s3 = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: await misconfig('s3_bucket_level_public_access_block', 'example-shop-assets') });
  assert.equal(s3.level, 'fix_in_infra');
  assert.deepEqual([s3.source.repo, s3.source.path, s3.source.via, s3.confirm], ['github.com/example-org/infra-terraform', 'envs/prod/storage', 'tag', false]);
  const helm = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: await misconfig('ocs-k8s-009', 'catalog-service') });
  assert.deepEqual([helm.level, helm.source.repo, helm.source.kind, helm.source.application], ['fix_in_infra', 'github.com/example-org/gitops', 'helm', 'catalog']);
  const inferred = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: await misconfig('ocs-k8s-005', 'order-service') });
  assert.deepEqual([inferred.level, inferred.confirm], ['fix_in_infra', true]);
  const appOnly = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: await misconfig('ocs-k8s-006', 'billing') });
  assert.equal(appOnly.level, 'needs_provenance');
  assert.match(appOnly.reason, /payments/);
  const untracked = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: 'misconfig:ocs-k8s-001:uid-5-node-debugger' });
  assert.equal(untracked.missingTags[0].key, 'argocd.argoproj.io/tracking-id|app.kubernetes.io/instance');
  assert.match(untracked.snippet.code, /resourceTrackingMethod: annotation/);
  const img = await mock.remediationOf({ kind: 'cloud_vulnerability', id: vuln('storefront-api') });
  assert.deepEqual([img.level, img.source.kind], ['fix_in_infra', 'dockerfile']);
  const noLabels = await mock.remediationOf({ kind: 'cloud_vulnerability', id: vuln('catalog-service') });
  assert.equal(noLabels.level, 'needs_provenance');
  assert.deepEqual(
    noLabels.missingTags.map((m) => m.key),
    ['org.opencontainers.image.source|git.repository_url', 'org.opencontainers.image.revision|git.commit.sha'],
  );
  assert.match(noLabels.snippet.code, /-t registry\.example\.com\/catalog-service/);
  assert.equal((await mock.remediationOf({ kind: 'cloud_vulnerability', id: vuln('i-0a1b', 'host') })).level, 'explain');
});

test('an infrastructure fix: the S3 block without an LLM, a confirmation for an inferred source, Helm values by Assist', async () => {
  const s3 = await mock.fixFinding({ kind: 'cloud_misconfiguration', id: await misconfig('s3_bucket_level_public_access_block', 'example-shop-assets') });
  assert.deepEqual([s3.method, s3.status, s3.source.located.address], ['editor', 'ready', 'aws_s3_bucket.assets']);
  assert.match(s3.change.files[0].after, /restrict_public_buckets = true/);
  const opened = await mock.openMergeRequest(s3.id);
  assert.equal(opened.mergeRequest.state, 'open');
  const order = await misconfig('ocs-k8s-005', 'order-service');
  await assert.rejects(mock.fixFinding({ kind: 'cloud_misconfiguration', id: order }), /confirm/);
  const confirmed = await mock.fixFinding({ kind: 'cloud_misconfiguration', id: order, confirmed: true });
  assert.deepEqual([confirmed.method, confirmed.source.confirmed], ['assist', true]);
  const helm = await mock.fixFinding({ kind: 'cloud_misconfiguration', id: await misconfig('ocs-k8s-009', 'catalog-service') });
  await sleep(4500);
  const done = await mock.getFix(helm.id);
  assert.equal(done.status, 'concluded');
  assert.equal(done.change.files[0].path, 'apps/catalog/values-prod.yaml');
  await assert.rejects(mock.fixFinding({ kind: 'cloud_misconfiguration', id: await misconfig('ocs-k8s-006', 'billing') }), /payments/);
});

test('tags are added from Tag Coverage to the repository picked', async () => {
  const fix = await mock.addTags({ resourceId: 'arn:aws:s3:::example-shop-logs', repository: 'github.com/example-org/infra-terraform', path: 'envs/prod/logging' });
  assert.deepEqual([fix.method, fix.findingKind, fix.source.path], ['editor', 'resource', 'envs/prod/logging']);
  assert.match(fix.change.files[0].after, /tf_path    = "envs\/prod\/logging"/);
  assert.equal((await mock.remediationOf({ kind: 'resource', id: 'arn:aws:s3:::example-shop-logs' })).fix.id, fix.id);
  await assert.rejects(mock.addTags({ resourceId: 'shop-prod-eu-dep-workflow-engine', repository: 'github.com/example-org/k8s-apps', path: '' }), /Argo CD/);
  await assert.rejects(mock.addTags({ resourceId: 'arn:aws:s3:::example-shop-logs', repository: 'git.example.com/x/y', path: '' }), /No GitHub or GitLab connection/);
});
