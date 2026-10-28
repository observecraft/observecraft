// API keys list filtering (Organization Settings → API Keys).

export type KeyStatus = 'active' | 'revoked' | 'all';

interface KeyLike {
  id: string;
  name: string;
  prefix: string;
  revokedAt?: number | null;
}

/** Keys of the status, matching the text in their name, ID or prefix. */
export function filterKeys<K extends KeyLike>(keys: K[], status: KeyStatus, text: string): K[] {
  const t = text.trim().toLowerCase();
  return keys.filter((k) => (status === 'all' || (status === 'revoked') === Boolean(k.revokedAt)) && (!t || [k.name, k.id, k.prefix].some((v) => v.toLowerCase().includes(t))));
}
