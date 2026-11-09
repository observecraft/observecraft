// oc-browser: OC_BROWSER_TOKEN (required), OC_BROWSER_PORT (8095), OC_BROWSER_CONCURRENCY (2),
// OC_BROWSER_SESSIONS (2, agent sessions at once), OC_SYNTHETICS_ALLOW_PRIVATE (false: internal addresses are refused),
// OC_BROWSER_EXECUTABLE (the image's Chromium).
import { createRunner } from './run.ts';
import { createApp } from './server.ts';
import { createSessions } from './session.ts';

const token = process.env.OC_BROWSER_TOKEN ?? '';
if (token.length < 16) {
  console.error('OC_BROWSER_TOKEN must be set (at least 16 characters)');
  process.exit(1);
}
const allowPrivate = process.env.OC_SYNTHETICS_ALLOW_PRIVATE === 'true';
const executablePath = process.env.OC_BROWSER_EXECUTABLE || undefined;
const runner = await createRunner({ allowPrivate, executablePath });
const sessions = await createSessions({ allowPrivate, executablePath, max: Math.max(1, Number(process.env.OC_BROWSER_SESSIONS ?? 2)) });
const port = Number(process.env.OC_BROWSER_PORT ?? 8095);
const server = createApp({ runner, sessions, token, concurrency: Math.max(1, Number(process.env.OC_BROWSER_CONCURRENCY ?? 2)) });
server.listen(port, () => console.log(`oc-browser listening on :${port}`));

for (const signal of ['SIGTERM', 'SIGINT'] as const) {
  process.on(signal, () => {
    server.close();
    void Promise.all([runner.close(), sessions.close()]).finally(() => process.exit(0));
  });
}
