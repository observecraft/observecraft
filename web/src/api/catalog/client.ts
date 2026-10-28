// Infrastructure → Catalog data access. In "live" mode resources come from the Rust API (hosts from
// host metrics and Agent metadata, Kubernetes and ECS resources from the collectors); otherwise the
// in-browser mock answers.
//   POST /api/v1/infrastructure/catalog   searchCatalog
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/catalog';

const live = API_MODE === 'live';

export type CatalogType = components['schemas']['CatalogType'];
export type CatalogResource = components['schemas']['CatalogResource'];
export type CatalogSearchRequest = components['schemas']['CatalogSearchRequest'];
export type CatalogPage = components['schemas']['CatalogPage'];
export type CatalogFacet = components['schemas']['CatalogFacet'];
export type CatalogCategory = components['schemas']['CatalogCategory'];
export type CatalogGroup = components['schemas']['CatalogGroup'];

export async function searchCatalog(req: CatalogSearchRequest): Promise<CatalogPage> {
  if (!live) return mock.searchCatalog(req);
  return post('/api/v1/infrastructure/catalog', req);
}
