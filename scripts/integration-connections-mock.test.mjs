// Mock mode (make web-mock) shows an integration set up on its own page as installed exactly while it has a saved
// connection or account, as the server does (`CONNECTIONS` in routes/integration_installs.rs for platform
// integrations, `connectors::connected` for connectors: native ones and packages).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  stdin: {
    contents: [
      "export { listCatalog } from './api/mock/integrations';",
      "export * as authentik from './api/mock/authentik';",
      "export * as fleetdm from './api/mock/fleetdm';",
      "export * as gitlabAudit from './api/mock/gitlabAudit';",
      "export * as googleWorkspace from './api/mock/googleWorkspace';",
      "export * as connectors from './api/mock/connectors';",
      "export * as aws from './api/mock/aws';",
      "export * as azure from './api/mock/azure';",
      "export * as entra from './api/mock/entra';",
      "export * as gcp from './api/mock/gcp';",
      "export * as slack from './api/mock/slack';",
    ].join('\n'),
    resolveDir: fileURLToPath(new URL('../web/src', import.meta.url)),
    loader: 'ts',
  },
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
  // Vite's lazy imports (agent specs, details) aren't needed here.
  define: { 'import.meta.env': '{}', 'import.meta.glob': 'globalThis.__noGlob' },
  logLevel: 'error',
});
globalThis.__noGlob = () => ({});
const web = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);

const status = async (id) => (await web.listCatalog()).find((i) => i.id === id)?.status;
const token = 'mock-token-of-twenty-characters';

/** Each connection the server counts: connect, then remove everything connected (the seeded demo accounts included). */
const CASES = {
  authentik: { connect: () => web.authentik.saveAuthentik({ baseUrl: 'https://auth.example.com', token }), remove: () => web.authentik.deleteAuthentik() },
  fleetdm: { connect: () => web.fleetdm.saveFleetdm({ baseUrl: 'https://fleet.example.com', token }), remove: () => web.fleetdm.deleteFleetdm() },
  'gitlab-audit-events': { connect: () => web.gitlabAudit.saveGitlabAudit({ baseUrl: 'https://gitlab.example.com', token, scope: 'instance' }), remove: () => web.gitlabAudit.deleteGitlabAudit() },
  'google-workspace': {
    connect: () =>
      web.googleWorkspace.saveGoogleWorkspace({
        serviceAccountKey: JSON.stringify({ type: 'service_account', client_email: 'reports@example-project.iam.gserviceaccount.com', private_key: 'test-only' }),
        subject: 'admin@example.com',
        applications: [],
      }),
    remove: () => web.googleWorkspace.deleteGoogleWorkspace(),
  },
  aws: { remove: async () => Promise.all((await web.aws.listAwsAccounts()).map((a) => web.aws.deleteAwsAccount(a.id))) },
  // Before azure: removing its app registrations removes their Entra ID logs too.
  'azure-active-directory': { remove: () => web.entra.saveEntraLogs('3c2b1a09-8f7e-4d6c-9b5a-4f3e2d1c0b9a', { enabled: false }) },
  azure: { remove: async () => Promise.all((await web.azure.listAzureAccounts()).map((a) => web.azure.deleteAzureAccount(a.id))) },
  'google-cloud': { remove: async () => Promise.all((await web.gcp.listGcpAccounts()).map((a) => web.gcp.deleteGcpAccount(a.id))) },
  slack: { remove: async () => Promise.all((await web.slack.getSlackOverview()).accounts.map((a) => web.slack.disconnectSlackAccount(a.id))) },
};

// Every bundled connector package, through the generic connectors mock, with its own mock connection.
const PACKAGES = JSON.parse(readFileSync(new URL('../web/src/api/connectors/packages.generated.json', import.meta.url), 'utf8'));
for (const { entry, mock } of PACKAGES) {
  if (entry.push) continue;
  CASES[entry.id] = {
    connect: () => web.connectors.createConnection(entry.id, { name: '', config: mock?.config ?? {}, secrets: Object.fromEntries((mock?.secrets ?? []).map((k) => [k, token])) }),
    remove: async () => Promise.all((await web.connectors.getConnector(entry.id)).connections.map((c) => web.connectors.deleteConnection(entry.id, c.id))),
  };
}

test('every connection the server counts as installed is covered here', async () => {
  const rust = readFileSync(new URL('../server/crates/oc-api/src/routes/integration_installs.rs', import.meta.url), 'utf8');
  const block = rust.match(/const CONNECTIONS[^=]*= \[([\s\S]*?)\];/)[1];
  const platform = [...block.matchAll(/\("([^"]+)", "[^"]+"\)/g)].map((m) => m[1]);
  // Connectors: the native ones (connectors/native.rs) and the bundled packages.
  const native = readFileSync(new URL('../server/crates/oc-api/src/connectors/native.rs', import.meta.url), 'utf8');
  const natives = [...native.matchAll(/native\(\s*"([^"]+)"/g)].map((m) => m[1]);
  const packages = (await web.connectors.listConnectors()).connectors.filter((c) => c.kind === 'package').map((c) => c.id);
  assert.deepEqual([...platform, ...natives, ...packages].sort(), Object.keys(CASES).sort());
});

for (const [id, c] of Object.entries(CASES)) {
  test(`${id} is installed while it has a connection`, async () => {
    if (c.connect) {
      assert.equal(await status(id), 'available');
      await c.connect();
    }
    assert.equal(await status(id), 'installed');
    await c.remove();
    assert.equal(await status(id), 'available');
  });
}
