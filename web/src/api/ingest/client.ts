// Ingest API keys: what applications, collectors and agents send telemetry with (org admins).
// Outside live mode the in-browser mock (mock/identity.ts) answers; secrets are shown only at creation.
import type { components } from '../generated/schema';
import { API_MODE, del, get, post } from '../http';
import * as mock from '../mock/identity';

export type IngestKey = components['schemas']['IngestKey'];

const live = API_MODE === 'live';

export const listIngestKeys = () => (live ? get('/api/v1/ingest-keys', {}) : mock.listIngestKeys());
export const createIngestKey = (name: string) => (live ? post('/api/v1/ingest-keys', { name }) : mock.createIngestKey(name));
export const revokeIngestKey = (id: string) => (live ? del('/api/v1/ingest-keys/{id}', { id }) : mock.revokeIngestKey(id));
