// Integrations → GitLab Audit Events: the connection the server polls a GitLab instance's (or group's, project's)
// audit events with; they arrive as logs (`source:gitlab-audit-events`).
//   GET    /api/v1/integrations/gitlab-audit-events        gitlabAuditStatus
//   PUT    /api/v1/integrations/gitlab-audit-events        saveGitlabAudit (admins)
//   DELETE /api/v1/integrations/gitlab-audit-events        deleteGitlabAudit (admins)
//   POST   /api/v1/integrations/gitlab-audit-events/test   testGitlabAudit (admins)
//   POST   /api/v1/integrations/gitlab-audit-events/sync   syncGitlabAudit (admins)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/gitlabAudit';

type S = components['schemas'];
export type GitlabAuditInput = S['GitlabAuditInput'];
export type GitlabAuditStatus = S['GitlabAuditStatus'];
export type GitlabAuditTest = S['GitlabAuditTest'];

export async function gitlabAuditStatus(): Promise<GitlabAuditStatus> {
  if (API_MODE !== 'live') return mock.gitlabAuditStatus();
  return get('/api/v1/integrations/gitlab-audit-events', {});
}

export async function saveGitlabAudit(input: GitlabAuditInput): Promise<GitlabAuditStatus> {
  if (API_MODE !== 'live') return mock.saveGitlabAudit(input);
  return put('/api/v1/integrations/gitlab-audit-events', {}, input);
}

export async function deleteGitlabAudit(): Promise<void> {
  if (API_MODE !== 'live') return mock.deleteGitlabAudit();
  return del('/api/v1/integrations/gitlab-audit-events', {});
}

export async function testGitlabAudit(input: GitlabAuditInput): Promise<GitlabAuditTest> {
  if (API_MODE !== 'live') return mock.testGitlabAudit(input);
  return post('/api/v1/integrations/gitlab-audit-events/test', input);
}

export async function syncGitlabAudit(): Promise<void> {
  if (API_MODE !== 'live') return mock.syncGitlabAudit();
  await postTo('/api/v1/integrations/gitlab-audit-events/sync', {}, undefined);
}
