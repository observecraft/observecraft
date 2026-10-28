// The release notes page reads CHANGELOG.md (written by release-please) as releases.

export interface ReleaseItem {
  /** The conventional-commit scope (`metrics`), when the entry has one. */
  scope: string | null;
  /** The entry's Markdown text (links to the PR and commit included). */
  text: string;
}

export interface Release {
  version: string;
  /** ISO date (YYYY-MM-DD), or '' when the heading has none. */
  date: string;
  /** The compare link of the release heading. */
  url: string | null;
  sections: { title: string; items: ReleaseItem[] }[];
}

/** `## [0.2.2](compare-url) (2026-10-27)` → its version, link and date. */
const RELEASE = /^##\s+\[?([^\]\s]+)\]?(?:\(([^)]+)\))?\s*(?:\((\d{4}-\d{2}-\d{2})\))?/;

/** The changelog's releases, newest first as written, with their sections and entries. */
export function parseChangelog(md: string): Release[] {
  const releases: Release[] = [];
  for (const line of md.split('\n')) {
    const release = RELEASE.exec(line);
    if (release) {
      releases.push({ version: release[1], url: release[2] ?? null, date: release[3] ?? '', sections: [] });
      continue;
    }
    const current = releases[releases.length - 1];
    if (!current) continue;
    const section = /^###\s+(.+)$/.exec(line);
    if (section) {
      current.sections.push({ title: section[1].trim(), items: [] });
      continue;
    }
    const item = /^\s*[*-]\s+(.+)$/.exec(line);
    if (item && current.sections.length) {
      const scoped = /^\*\*([^*:]+):\*\*\s*(.+)$/.exec(item[1]);
      current.sections[current.sections.length - 1].items.push(scoped ? { scope: scoped[1], text: scoped[2] } : { scope: null, text: item[1] });
    }
  }
  return releases;
}
