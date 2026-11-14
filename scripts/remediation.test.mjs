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
  const cloud = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: 'misconfig:x' });
  assert.equal(cloud.level, 'needs_provenance');
  assert.equal(cloud.taggingPolicies, '/organization-settings/tagging-policies');
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
  const run = await mock.explainFinding({ kind: 'cloud_misconfiguration', id: 'misconfig:x' });
  await sleep(4500);
  const done = await mock.getFix(fix.id);
  assert.equal(done.status, 'concluded');
  assert.ok(done.change.files[0].after.includes('secret store'));
  const r = await mock.remediationOf({ kind: 'cloud_misconfiguration', id: 'misconfig:x' });
  assert.equal(r.explainRun.id, run.id);
  assert.equal(r.explainRun.verdict, 'explained');
});
