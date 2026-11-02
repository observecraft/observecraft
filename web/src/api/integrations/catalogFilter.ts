import type { CatalogRequest, IntegrationSummary } from './types';

/** The catalog entries a search asks for (text in the name or a category, status, category, repository), by name. */
export function filterCatalog(entries: IntegrationSummary[], req: CatalogRequest): IntegrationSummary[] {
  const q = req.query.trim().toLowerCase();
  return entries
    .filter((s) => (req.status === 'all' ? true : req.status === 'installed' ? s.status === 'installed' || s.status === 'warning' : s.status === req.status))
    .filter((s) => !req.category || s.categories.includes(req.category))
    .filter((s) => !req.repository || s.repository === req.repository)
    .filter((s) => !q || s.name.toLowerCase().includes(q) || s.categories.some((c) => c.toLowerCase().includes(q)))
    .sort((a, b) => a.name.localeCompare(b.name));
}
