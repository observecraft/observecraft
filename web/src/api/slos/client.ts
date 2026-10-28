// Service level objectives and their corrections. In "live" mode they come from the Rust API
// (Postgres definitions, SLIs computed from metrics and monitor history); otherwise the in-browser
// mock answers (mock/slos.ts).
//   POST   /api/v1/slos/search           searchSlos
//   GET    /api/v1/slos/:id?window=      getSlo
//   POST   /api/v1/slos                  createSlo
//   PUT    /api/v1/slos/:id              updateSlo
//   DELETE /api/v1/slos/:id              deleteSlo
//   GET    /api/v1/slo-corrections       listCorrections
//   POST   /api/v1/slo-corrections       createCorrection
//   PUT    /api/v1/slo-corrections/:id   updateCorrection
//   DELETE /api/v1/slo-corrections/:id   deleteCorrection
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/slos';
import type { SloCorrection, SloCorrectionInput, SloDetail, SloInput, SloItem, SloSearchPage, SloSearchRequest, SloWindow } from './types';

export * from './types';

const live = API_MODE === 'live';

export const searchSlos = (req: SloSearchRequest): Promise<SloSearchPage> => (live ? post('/api/v1/slos/search', req) : mock.searchSlos(req));
export const getSlo = (id: string, window?: SloWindow): Promise<SloDetail> => (live ? get('/api/v1/slos/{id}', { id }, undefined, window ? { window } : undefined) : mock.getSlo(id, window));
export const createSlo = (input: SloInput): Promise<SloItem> => (live ? post('/api/v1/slos', input) : mock.createSlo(input));
export const updateSlo = (id: string, input: SloInput): Promise<SloItem> => (live ? put('/api/v1/slos/{id}', { id }, input) : mock.updateSlo(id, input));
export const deleteSlo = (id: string): Promise<void> => (live ? del('/api/v1/slos/{id}', { id }) : mock.deleteSlo(id));
export const listCorrections = (sloId?: string): Promise<SloCorrection[]> => (live ? get('/api/v1/slo-corrections', {}, undefined, sloId ? { sloId } : undefined) : mock.listCorrections(sloId));
export const createCorrection = (input: SloCorrectionInput): Promise<SloCorrection> => (live ? post('/api/v1/slo-corrections', input) : mock.createCorrection(input));
export const updateCorrection = (id: string, input: SloCorrectionInput): Promise<SloCorrection> => (live ? put('/api/v1/slo-corrections/{id}', { id }, input) : mock.updateCorrection(id, input));
export const deleteCorrection = (id: string): Promise<void> => (live ? del('/api/v1/slo-corrections/{id}', { id }) : mock.deleteCorrection(id));
