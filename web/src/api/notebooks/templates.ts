// Built-in notebook templates ("Get started fast" and the Templates tab). Each one is a title, tags
// and starter cells; creating a notebook from a template copies them with fresh cell ids. The text
// is ObserveCraft's own.
import type { CellDefinition, NotebookCell, NotebookTime } from './types';

export interface NotebookTemplate {
  id: 'postmortem' | 'runbook' | 'incident-report' | 'investigation';
  name: string;
  description: string;
  /** Default title of a notebook made from it; `{subject}` is replaced by the subject when given. */
  title: string;
  tags: string[];
  time: NotebookTime;
  cells: CellDefinition[];
}

const md = (text: string): CellDefinition => ({ type: 'markdown', text });

export const NOTEBOOK_TEMPLATES: NotebookTemplate[] = [
  {
    id: 'postmortem',
    name: 'Postmortem',
    description: 'A blameless review of an incident: impact, timeline, root cause and follow-ups.',
    title: 'Postmortem: {subject}',
    tags: ['postmortem'],
    time: '1d',
    cells: [
      md('# Summary\nWhat happened, in two or three sentences, and who was affected.\n\n- **Severity:** \n- **Duration:** \n- **Detected by:** monitor / customer report / on-call'),
      md('## Impact\nWhich customers, requests or orders were affected, and for how long.'),
      { type: 'timeseries', title: 'Error rate during the incident', display: 'line', requests: [{ query: 'avg:trace.error_rate{env:prod} by {service}' }] },
      md('## Timeline\n1. **Detection** — \n2. **Escalation** — \n3. **Mitigation** — \n4. **Resolution** — '),
      md('## Root cause\nWhat failed, and why our safeguards did not catch it earlier.'),
      md('## What went well, what did not\n- \n- '),
      md('## Follow-ups\n- [ ] \n- [ ] '),
    ],
  },
  {
    id: 'runbook',
    name: 'Runbook',
    description: 'Step-by-step checks for an alert, with the graphs and logs to look at.',
    title: 'Runbook: {subject}',
    tags: ['runbook'],
    time: '4h',
    cells: [
      md('# When to use this runbook\nThe alert or symptom this runbook answers, and the service it covers.'),
      md('## 1. Check the traffic and errors'),
      { type: 'timeseries', title: 'Requests', display: 'bars', requests: [{ query: 'sum:trace.requests{env:prod} by {service}' }] },
      { type: 'toplist', title: 'Services with the most errors', request: { query: 'sum:trace.errors{env:prod} by {service}' }, aggregator: 'sum', limit: 10 },
      md('## 2. Read the latest errors'),
      { type: 'log_stream', title: 'Error logs', query: 'status:error env:prod', limit: 25 },
      md(
        '## 3. Mitigate\n- Roll back the latest deployment if it lines up with the start of the errors.\n- Scale out the service if it is saturated.\n\n## 4. Escalate\nWho to page when the steps above do not help.',
      ),
    ],
  },
  {
    id: 'incident-report',
    name: 'Incident Report',
    description: 'A live status document for responders and stakeholders while an incident is open.',
    title: 'Incident report: {subject}',
    tags: ['incident'],
    time: '1h',
    cells: [
      md('# Status\n**Investigating** — what we know so far, updated as the incident evolves.\n\n- **Commander:** \n- **Communications:** \n- **Next update:** '),
      {
        type: 'query_value',
        title: 'Current error rate (%)',
        request: { query: 'avg:trace.error_rate{env:prod}' },
        aggregator: 'avg',
        precision: 2,
        unit: '%',
        formats: [
          { comparator: '>', value: 5, palette: 'red' },
          { comparator: '<=', value: 5, palette: 'green' },
        ],
      },
      { type: 'timeseries', title: 'Latency (p95)', display: 'line', requests: [{ query: 'avg:trace.latency.p95{env:prod} by {service}' }] },
      md('## Customer impact\nWhat customers see, and the workaround if there is one.'),
      md('## Actions taken\n- '),
    ],
  },
  {
    id: 'investigation',
    name: 'Investigation',
    description: 'Collect graphs, logs and notes while you dig into a question about your systems.',
    title: 'Investigation: {subject}',
    tags: ['investigation'],
    time: '4h',
    cells: [
      md('# Question\nWhat are we trying to find out, and why it matters.'),
      { type: 'timeseries', title: 'Latency (p95)', display: 'line', requests: [{ query: 'avg:trace.latency.p95{env:prod} by {service}' }] },
      { type: 'log_stream', title: 'Matching logs', query: 'env:prod status:error', limit: 25 },
      md('## Findings\n- \n\n## Next steps\n- '),
    ],
  },
];

let seq = 0;

/** A cell id unique within the session (`c-lx3k9a-1`). */
export function newCellId(): string {
  seq += 1;
  return `c-${Date.now().toString(36)}-${seq}`;
}

/** Cells for a notebook: copies of `definitions` with fresh ids. */
export function cellsFrom(definitions: CellDefinition[]): NotebookCell[] {
  return definitions.map((definition) => ({ id: newCellId(), definition: structuredClone(definition) }));
}

/** The title, tags, time range and cells of a new notebook made from `template`. */
export function fromTemplate(template: NotebookTemplate, subject = '') {
  const title = (subject.trim() ? template.title.replace('{subject}', subject.trim()) : `${template.name} (draft)`).slice(0, 200);
  return { title, tags: [...template.tags], time: template.time, cells: cellsFrom(template.cells) };
}
