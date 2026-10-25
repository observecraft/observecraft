// Ingest API keys: what applications, collectors and agents send telemetry with (org admins).
import type { components } from '../generated/schema';
import { del, get, post } from '../http';

export type IngestKey = components['schemas']['IngestKey'];

export const listIngestKeys = () => get('/api/v1/ingest-keys', {});
export const createIngestKey = (name: string) => post('/api/v1/ingest-keys', { name });
export const revokeIngestKey = (id: string) => del('/api/v1/ingest-keys/{id}', { id });
