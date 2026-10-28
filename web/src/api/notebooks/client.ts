// Notebooks data access. In "live" mode every call goes to the Rust API (notebooks in Postgres);
// otherwise the in-browser mock (mock/notebooks.ts) answers. Cell definitions are opaque JSON to
// the server; their schema lives in ./types.
import type { components } from '../generated/schema';
import { API_MODE, del, get, patch, post, postTo, put } from '../http';
import * as mock from '../mock/notebooks';
import type { CreateNotebookRequest, Notebook, NotebookCell, NotebookPatch, NotebookSearchPage, NotebookSearchRequest, NotebookSummary, NotebookTime } from './types';

export * from './types';

const live = API_MODE === 'live';

type S = components['schemas'];
type WireCells = S['Notebook']['cells'];

const toSummary = (n: S['NotebookSummary']): NotebookSummary => ({ ...n, author: n.author ?? null, time: n.time as NotebookTime, viewedAt: n.viewedAt ?? null, deletedAt: n.deletedAt ?? null });
const toNotebook = (n: S['Notebook']): Notebook => ({ ...toSummary(n), cells: n.cells as unknown as NotebookCell[] });
const wireCells = (cells: NotebookCell[] | undefined) => cells as unknown as WireCells | undefined;

export async function searchNotebooks(req: NotebookSearchRequest): Promise<NotebookSearchPage> {
  if (!live) return mock.searchNotebooks(req);
  const page = await post('/api/v1/notebooks/search', req);
  return { ...page, notebooks: page.notebooks.map(toSummary) };
}

/** Opens a notebook (recorded under Recents). */
export async function getNotebook(id: string): Promise<Notebook> {
  if (!live) return mock.getNotebook(id);
  return toNotebook(await get('/api/v1/notebooks/{id}', { id }));
}

export async function createNotebook(req: CreateNotebookRequest): Promise<Notebook> {
  if (!live) return mock.createNotebook(req);
  return toNotebook(await post('/api/v1/notebooks', { ...req, teams: req.teams ?? [], tags: req.tags ?? [], cells: wireCells(req.cells) ?? [] }));
}

export async function updateNotebook(id: string, p: NotebookPatch): Promise<Notebook> {
  if (!live) return mock.updateNotebook(id, p);
  return toNotebook(await patch('/api/v1/notebooks/{id}', { id }, { ...p, cells: wireCells(p.cells) }));
}

export async function cloneNotebook(id: string): Promise<Notebook> {
  if (!live) return mock.cloneNotebook(id);
  return toNotebook(await postTo('/api/v1/notebooks/{id}/clone', { id }, undefined));
}

/** Moves the notebook to Recently Deleted. */
export const deleteNotebook = (id: string): Promise<void> => (live ? del('/api/v1/notebooks/{id}', { id }) : mock.deleteNotebook(id));

export async function restoreNotebook(id: string): Promise<Notebook> {
  if (!live) return mock.restoreNotebook(id);
  return toNotebook(await postTo('/api/v1/notebooks/{id}/restore', { id }, undefined));
}

export const setNotebookFavorite = (id: string, favorite: boolean): Promise<void> => (live ? put('/api/v1/notebooks/{id}/favorite', { id }, { favorite }) : mock.setNotebookFavorite(id, favorite));
