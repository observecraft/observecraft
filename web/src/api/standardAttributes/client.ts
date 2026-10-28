// Standard attributes (Logs → Configuration → Standard Attributes): the org's attribute names, types
// and aliases, remapped on every ingested log. Outside live mode the in-browser mock (mock/standardAttributes.ts) answers.
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/standardAttributes';
import type { StandardAttribute, StandardAttributeInput } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listStandardAttributes = (): Promise<StandardAttribute[]> => (live ? get('/api/v1/logs/standard-attributes', {}) : mock.listStandardAttributes());
export const createStandardAttribute = (i: StandardAttributeInput): Promise<StandardAttribute> => (live ? post('/api/v1/logs/standard-attributes', i) : mock.createStandardAttribute(i));
export const updateStandardAttribute = (path: string, i: StandardAttributeInput): Promise<StandardAttribute> =>
  live ? put('/api/v1/logs/standard-attributes/{path}', { path }, i) : mock.updateStandardAttribute(path, i);
export const deleteStandardAttribute = (path: string): Promise<void> => (live ? del('/api/v1/logs/standard-attributes/{path}', { path }) : mock.deleteStandardAttribute(path));
/** Drops the org's changes; resolves to the default list. */
export const resetStandardAttributes = async (): Promise<StandardAttribute[]> => {
  if (!live) return mock.resetStandardAttributes();
  await del('/api/v1/logs/standard-attributes', {});
  return listStandardAttributes();
};
