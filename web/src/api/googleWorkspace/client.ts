// Integrations → Google Workspace: the connection the server reads a Workspace account's Reports API activities with
// (a service account acting as an administrator); they arrive as logs (`source:gsuite`, `service:` the application).
//   GET    /api/v1/integrations/google-workspace        googleWorkspaceStatus
//   PUT    /api/v1/integrations/google-workspace        saveGoogleWorkspace (admins)
//   DELETE /api/v1/integrations/google-workspace        deleteGoogleWorkspace (admins)
//   POST   /api/v1/integrations/google-workspace/test   testGoogleWorkspace (admins)
//   POST   /api/v1/integrations/google-workspace/sync   syncGoogleWorkspace (admins)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/googleWorkspace';

type S = components['schemas'];
export type GoogleWorkspaceInput = S['GoogleWorkspaceInput'];
export type GoogleWorkspaceStatus = S['GoogleWorkspaceStatus'];
export type GoogleWorkspaceTest = S['GoogleWorkspaceTest'];
export type GoogleWorkspaceSync = S['GoogleWorkspaceSync'];

export async function googleWorkspaceStatus(): Promise<GoogleWorkspaceStatus> {
  if (API_MODE !== 'live') return mock.googleWorkspaceStatus();
  return get('/api/v1/integrations/google-workspace', {});
}

export async function saveGoogleWorkspace(input: GoogleWorkspaceInput): Promise<GoogleWorkspaceStatus> {
  if (API_MODE !== 'live') return mock.saveGoogleWorkspace(input);
  return put('/api/v1/integrations/google-workspace', {}, input);
}

export async function deleteGoogleWorkspace(): Promise<void> {
  if (API_MODE !== 'live') return mock.deleteGoogleWorkspace();
  return del('/api/v1/integrations/google-workspace', {});
}

export async function testGoogleWorkspace(input: GoogleWorkspaceInput): Promise<GoogleWorkspaceTest> {
  if (API_MODE !== 'live') return mock.testGoogleWorkspace(input);
  return post('/api/v1/integrations/google-workspace/test', input);
}

export async function syncGoogleWorkspace(): Promise<void> {
  if (API_MODE !== 'live') return mock.syncGoogleWorkspace();
  await postTo('/api/v1/integrations/google-workspace/sync', {}, undefined);
}
