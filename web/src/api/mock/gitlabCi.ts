// In-browser stand-in for Software Delivery → Settings → GitLab CI (routes/gitlab_ci.rs). The connection
// lives in memory for the session; a token starting with `glpat-` is accepted. Once saved, the example-org
// projects are "followed" and the imported pipeline count grows with time, as the server's poller would.
import type { components } from '../generated/schema';

type S = components['schemas'];
type Input = S['GitlabCiInput'];
type Status = S['GitlabCiStatus'];
type Test = S['GitlabCiTest'];

const PROJECTS = ['example-org/storefront-api', 'example-org/order-service', 'example-org/billing', 'example-org/catalog-service', 'example-org/inventory-service', 'example-org/web-frontend'];
const MIN = 60_000;

let saved: { input: Input; at: number } | null = null;
let syncedAt = 0;

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

function fail(message: string): never {
  throw new Error(message);
}

function checked(input: Input): Input {
  const baseUrl = (input.baseUrl ?? '').trim().replace(/\/+$/, '') || 'https://gitlab.com';
  if (!/^https:\/\/[^\s/@]+/.test(baseUrl)) fail('The base URL must use https');
  const groupPath = (input.groupPath ?? '').trim().replace(/^\/+|\/+$/g, '');
  const projectPaths = [...new Set((input.projectPaths ?? []).map((p) => p.trim().replace(/^\/+|\/+$/g, '')).filter(Boolean))];
  if (!groupPath && projectPaths.length === 0 && !input.allProjects) fail('Choose a group or at least one project to follow');
  return { ...input, baseUrl, groupPath, projectPaths };
}

function followed(input: Input): string[] {
  const group = input.allProjects ? PROJECTS : input.groupPath ? PROJECTS.filter((p) => p.startsWith(`${input.groupPath}/`)) : [];
  return [...new Set([...group, ...(input.projectPaths ?? [])])].sort();
}

function status(): Status {
  if (!saved) return { connection: null, projects: [], discoveredAt: null, discoveryError: null, lastSyncAt: null, pipelinesImported: 0, jobsImported: 0, lastImportAt: null };
  const now = Date.now();
  const lastSync = Math.max(syncedAt, now - ((now - saved.at) % MIN));
  const projects = followed(saved.input).map((path, i) => ({ id: 4211 + i, path, lastSyncAt: lastSync, syncedUntil: lastSync - 2 * MIN, error: null }));
  // The first sync reads a week back, then a few pipelines a minute.
  const pipelines = projects.length * 40 + Math.floor((now - saved.at) / MIN) * projects.length;
  return {
    connection: {
      baseUrl: saved.input.baseUrl,
      groupPath: saved.input.groupPath ?? '',
      includeSubgroups: saved.input.includeSubgroups ?? true,
      projectPaths: saved.input.projectPaths ?? [],
      allProjects: saved.input.allProjects ?? false,
      updatedAt: saved.at,
    },
    projects,
    discoveredAt: saved.at,
    discoveryError: null,
    lastSyncAt: lastSync,
    pipelinesImported: pipelines,
    jobsImported: pipelines * 4,
    lastImportAt: lastSync,
  };
}

export const gitlabCiStatus = (): Promise<Status> => delay(status());

export function testGitlabCi(input: Input): Promise<Test> {
  const c = checked(input);
  const token = input.token?.trim() || (saved && saved.input.baseUrl === c.baseUrl ? 'glpat-saved' : '');
  if (!token) fail('GitLab needs an access token with the read_api scope');
  if (!token.startsWith('glpat-'))
    return delay({ ok: false, message: 'GitLab refused the token: The provider answered: the credentials were refused (401 Unauthorized)', user: null, scopes: [], projects: 0, sample: [] }, 400);
  const projects = followed(c);
  if (projects.length === 0) return delay({ ok: false, message: 'The token sees no project in this group', user: 'ci-reader', scopes: ['read_api'], projects: 0, sample: [] }, 400);
  return delay(
    {
      ok: true,
      message: `Connected as ci-reader: ${projects.length} project${projects.length === 1 ? '' : 's'} to follow`,
      user: 'ci-reader',
      scopes: ['read_api'],
      projects: projects.length,
      sample: projects.slice(0, 5),
    },
    400,
  );
}

export function saveGitlabCi(input: Input): Promise<Status> {
  const c = checked(input);
  if (!input.token?.trim() && !(saved && saved.input.baseUrl === c.baseUrl)) fail(saved ? 'Enter the token again: the GitLab URL changed' : 'GitLab needs an access token with the read_api scope');
  saved = { input: { ...c, token: null }, at: Date.now() };
  return delay(status(), 250);
}

export function deleteGitlabCi(): Promise<void> {
  if (!saved) fail('GitLab is not connected');
  saved = null;
  return delay(undefined);
}

export function syncGitlabCi(): Promise<void> {
  if (!saved) fail('GitLab is not connected');
  syncedAt = Date.now();
  return delay(undefined);
}
