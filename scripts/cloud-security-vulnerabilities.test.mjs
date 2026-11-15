import assert from 'node:assert/strict';
import test from 'node:test';

const { expandInfraVulnerabilities } = await import('../web/src/api/cloudSecurity/expand.ts');

test('the compact vulnerability list expands to one row per package with its advisory, resource and triage key', () => {
  const advisory = {
    advisoryId: 'OCSA-DEMO-2026-0105',
    cve: 'CVE-2026-90105',
    aliases: ['CVE-2026-90105'],
    title: 'example-musl: stack overflow',
    severity: 'critical',
    score: 9.8,
    cwes: [],
    exploitAvailable: false,
  };
  const image = {
    resourceId: 'sha256:abc',
    resourceName: 'registry.example.com/storefront-api:1.42.0',
    resourceKind: 'container_image',
    host: 'i-0a1b2c3d4e5f60001',
    env: 'prod',
    os: 'alpine 3.19',
    running: true,
    lastSeen: 2,
  };
  const row = { advisory: 0, resource: 0, ecosystem: 'Alpine:v3.19', package: 'example-musl', version: '1.2.4-r2', fixed: '1.2.4-r3', status: 'open', firstSeen: 1 };
  const [v] = expandInfraVulnerabilities({ advisories: [advisory], resources: [image], rows: [row] });
  assert.equal(v.key, 'csm-vuln:OCSA-DEMO-2026-0105:sha256:abc:example-musl');
  assert.deepEqual([v.title, v.resourceName, v.package, v.fixed, v.running], ['example-musl: stack overflow', image.resourceName, 'example-musl', '1.2.4-r3', true]);
  assert.equal('advisory' in v || 'resource' in v, false);
});
