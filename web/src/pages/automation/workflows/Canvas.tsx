import { useMemo, useState } from 'react';
import { Maximize2, Minus, Plus, Trash2 } from 'lucide-react';
import { allSteps, describeCron, STEP_LABEL, TRIGGER_LABEL, type BlockRef } from '../../../api/workflows/logic';
import type { WorkflowStep, WorkflowTrigger } from '../../../api/workflows/types';
import { layoutWorkflow, NODE_H, NODE_W, TRIGGER_ID } from './canvasLayout';
import { StepTile, TRIGGER_ICON } from './common';

/** One line under a step's title: what it does, in short. */
export function stepSummary(s: WorkflowStep): string {
  const f = (k: string) => s.fields[k] ?? '';
  switch (s.kind) {
    case 'http': {
      const r = s.requests[0];
      if (!r) return 'No request';
      const host = /^https?:\/\/([^/?#]+)/.exec(r.url)?.[1] ?? r.url;
      return `${r.method} ${host}${s.requests.length > 1 ? ` + ${s.requests.length - 1} more` : ''}`;
    }
    case 'notify':
      return f('recipients') || 'No recipients';
    case 'metric_query':
    case 'log_count':
      return f('query') || 'No query';
    case 'wait':
      return `${f('seconds') || '?'} seconds`;
    case 'if':
      return s.expression || 'No condition';
    case 'for_each':
      return `For each item of ${s.expression || '?'}`;
    case 'create_incident':
      return `${f('severity') || 'SEV-3'} · ${f('title')}`;
    case 'page_on_call':
      return `@${f('team').replace(/^@/, '')}`;
    case 'incident_comment':
      return `Incident ${f('incident')}`;
    case 'synthetic_run':
      return `Test ${f('test')}`;
  }
}

export function triggerSummary(triggers: WorkflowTrigger[], handle: string): string {
  if (!triggers.length) return 'No trigger: add one';
  return triggers.map((t) => (t.kind === 'schedule' ? describeCron(t.cron, t.timezone) : t.kind === 'monitor' ? `@workflow-${handle || '…'}` : TRIGGER_LABEL[t.kind])).join(' · ');
}

interface Props {
  steps: WorkflowStep[];
  triggers: WorkflowTrigger[];
  handle: string;
  selected: string | null;
  onSelect: (id: string | null) => void;
  /** Editing: "+" insertion points and deleting. */
  onAdd?: (ref: BlockRef, index: number) => void;
  onDelete?: (id: string) => void;
  /** Run view: the status of each step that ran. */
  statuses?: Record<string, 'succeeded' | 'failed'>;
  /** The insertion point the catalog will add to. */
  adding?: { ref: BlockRef; index: number } | null;
}

/** The workflow canvas: a dotted background and the vertical flow from the trigger down. */
export function Canvas({ steps, triggers, handle, selected, onSelect, onAdd, onDelete, statuses, adding }: Props) {
  const [zoom, setZoom] = useState(1);
  const layout = useMemo(() => layoutWorkflow(steps), [steps]);
  const byId = useMemo(() => new Map(allSteps(steps).map((s) => [s.id, s])), [steps]);
  const pad = 60;
  const w = layout.width + 2 * pad;
  const h = layout.height + 2 * pad;
  return (
    <div className="wfe-canvas-wrap">
      <div className="wfe-canvas" onClick={() => onSelect(null)}>
        <div className="wfe-canvas-inner" style={{ width: w * zoom, height: h * zoom }}>
          <div className="wfe-stage" style={{ width: w, height: h, transform: `scale(${zoom})` }}>
            {layout.frames.map((f) => (
              <div key={f.id} className="wfe-frame" style={{ left: f.x + pad, top: f.y + pad, width: f.w, height: f.h }} />
            ))}
            <svg className="wfe-edges" width={w} height={h}>
              {layout.edges.map((e, i) => (
                <polyline key={i} points={e.map(([x, y]) => `${x + pad},${y + pad}`).join(' ')} />
              ))}
            </svg>
            {layout.labels.map((l, i) => (
              <span key={i} className={`wfe-branch-label wfe-branch-${l.text}`} style={{ left: l.x + pad, top: l.y + pad }}>
                {l.text}
              </span>
            ))}
            {layout.nodes.map((n) => {
              const isTrigger = n.id === TRIGGER_ID;
              const s = byId.get(n.id);
              const status = statuses?.[n.id];
              const ran = statuses && !isTrigger ? (status ? ` wfe-ran-${status}` : ' wfe-not-ran') : '';
              return (
                <div
                  key={n.id}
                  className={`wfe-node${selected === n.id ? ' selected' : ''}${isTrigger ? ' wfe-node-trigger' : ''}${ran}`}
                  style={{ left: n.x + pad, top: n.y + pad, width: NODE_W, height: NODE_H }}
                  onClick={(e) => {
                    e.stopPropagation();
                    onSelect(n.id);
                  }}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => e.key === 'Enter' && onSelect(n.id)}
                >
                  {isTrigger ? <span className="wf-tile wf-tone-orange">{TRIGGER_ICON[triggers[0]?.kind ?? 'manual']}</span> : s && <StepTile kind={s.kind} />}
                  <div className="wfe-node-text">
                    <div className="wfe-node-title">
                      {isTrigger ? (triggers.length ? `${triggers.map((t) => TRIGGER_LABEL[t.kind]).join(', ')} trigger` : 'Add a trigger') : s?.name || (s ? STEP_LABEL[s.kind] : n.id)}
                    </div>
                    <div className="wfe-node-sub">{isTrigger ? triggerSummary(triggers, handle) : s ? stepSummary(s) : ''}</div>
                  </div>
                  {onDelete && !isTrigger && (
                    <button
                      className="wfe-node-delete"
                      aria-label={`Delete ${n.id}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        onDelete(n.id);
                      }}
                    >
                      <Trash2 size={13} />
                    </button>
                  )}
                  {status && <span className={`wfe-node-dot wfe-dot-${status}`} />}
                </div>
              );
            })}
            {onAdd &&
              layout.adds.map((a, i) => {
                const active = adding && adding.ref.parent === a.ref.parent && adding.ref.branch === a.ref.branch && adding.index === a.index;
                return (
                  <button
                    key={i}
                    className={`wfe-add${active ? ' active' : ''}`}
                    style={{ left: a.x + pad, top: a.y + pad }}
                    aria-label="Add a step"
                    onClick={(e) => {
                      e.stopPropagation();
                      onAdd(a.ref, a.index);
                    }}
                  >
                    <Plus size={12} />
                  </button>
                );
              })}
          </div>
        </div>
      </div>
      <div className="wfe-zoom" onClick={(e) => e.stopPropagation()}>
        <button aria-label="Zoom out" onClick={() => setZoom((z) => Math.max(0.4, +(z - 0.1).toFixed(2)))}>
          <Minus size={14} />
        </button>
        <span>{Math.round(zoom * 100)}%</span>
        <button aria-label="Zoom in" onClick={() => setZoom((z) => Math.min(1.6, +(z + 0.1).toFixed(2)))}>
          <Plus size={14} />
        </button>
        <button aria-label="Reset zoom" onClick={() => setZoom(1)}>
          <Maximize2 size={13} />
        </button>
      </div>
    </div>
  );
}
