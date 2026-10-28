import type { components } from '../generated/schema';

type S = components['schemas'];
export type ReferenceTable = S['ReferenceTable'];
export type ReferenceTableInput = S['ReferenceTableInput'];
export type ReferenceTableFile = S['ReferenceTableFile'];
export type ReferenceTableRows = S['ReferenceTableRows'];

/** A page of a table's rows, optionally searched (in one column, or every column). */
export interface ReferenceTableRowsQuery {
  query?: string;
  column?: string;
  limit: number;
  offset: number;
}
