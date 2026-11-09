import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, Check, ChevronLeft, ChevronRight, Film, ImageOff, Minus, MonitorSmartphone, X } from 'lucide-react';
import type { BrowserStepResult, SyntheticRunDetail } from '../../api/synthetics/client';
import { DEVICES, FAILURE_LABEL, STEP_LABEL } from '../../api/synthetics/browserSteps';
import { fmtMs } from './common';

/**
 * One browser run: what failed, the device and browser, the run's video and each step with its duration, URL,
 * errors and screenshot (opened large on click). `shot(i)` is the screenshot of step i (0 = the start URL).
 */
export function BrowserRunView({ run, shot, video }: { run: SyntheticRunDetail; shot: (i: number) => string | null; video?: string | null }) {
  const b = run.browser;
  const [open, setOpen] = useState<number | null>(null);
  if (!b) return <p className="syn-run-error">{run.error ?? 'The run has no browser result.'}</p>;
  const device = DEVICES.find((d) => d.id === b.device.id);
  const done = b.steps.filter((s) => s.status === 'passed').length;
  const shots = b.steps.map((s, i) => (s.hasScreenshot ? shot(i) : null));
  return (
    <div className="syn-run syn-br">
      {!run.passed && run.error && (
        <p className="syn-run-error">
          {b.failure && <b>{FAILURE_LABEL[b.failure.code] ?? b.failure.code}: </b>}
          {run.error}
        </p>
      )}
      <div className="syn-run-facts">
        <div>
          <label>Starting URL</label>
          <span className="mono syn-wrap">{b.startUrl}</span>
        </div>
        <div>
          <label>Device</label>
          <span className="syn-icon-text">
            <MonitorSmartphone size={14} /> {device?.label ?? b.device.id} ({b.device.width} × {b.device.height})
          </span>
        </div>
        <div>
          <label>Browser</label>
          <b>Chromium {b.browserVersion}</b>
        </div>
        <div>
          <label>Steps</label>
          <b>
            {done} of {b.steps.length} passed
          </b>
        </div>
        <div>
          <label>Duration</label>
          <b>{fmtMs(run.durationMs)}</b>
        </div>
      </div>
      {video && (
        <section className="syn-run-section syn-br-video">
          <h4>
            <Film size={15} /> Video
          </h4>
          <video src={video} controls preload="metadata" aria-label="Video of the run" />
        </section>
      )}
      <ol className="syn-br-steps">
        {b.steps.map((s, i) => (
          <StepRow key={i} step={s} index={i} shot={shots[i]} onOpen={() => setOpen(i)} />
        ))}
      </ol>
      {open !== null && <Lightbox steps={b.steps} shots={shots} at={open} onMove={setOpen} onClose={() => setOpen(null)} />}
    </div>
  );
}

function StatusIcon({ status, allowed }: { status: string; allowed: boolean }) {
  if (status === 'passed') return <Check size={14} />;
  if (status === 'skipped') return <Minus size={14} />;
  return allowed ? <AlertTriangle size={14} /> : <X size={14} />;
}

function StepRow({ step: s, index, shot, onOpen }: { step: BrowserStepResult; index: number; shot: string | null; onOpen: () => void }) {
  const [errors, setErrors] = useState(false);
  const browserErrors = s.browserErrors ?? [];
  return (
    <li className={`syn-br-step ${s.status}${s.status === 'failed' && s.allowFailure ? ' allowed' : ''}`}>
      <span className="syn-br-step-icon" title={s.status}>
        <StatusIcon status={s.status} allowed={!!s.allowFailure} />
      </span>
      <div className="syn-br-step-main">
        <div className="syn-br-step-title">
          <span className="syn-br-step-n">{index}</span>
          <b>{s.name}</b>
          <span className="syn-chip">{STEP_LABEL[s.type as keyof typeof STEP_LABEL] ?? s.type}</span>
          {s.allowFailure && <span className="muted">may fail{s.isCritical ? ', critical' : ''}</span>}
        </div>
        {s.status !== 'skipped' && (
          <div className="syn-br-step-meta muted">
            <span>{fmtMs(s.durationMs)}</span>
            {s.url && <span className="mono syn-wrap">{s.url}</span>}
          </div>
        )}
        {s.error && (
          <p className="syn-br-step-error">
            {s.failure && <b>{FAILURE_LABEL[s.failure.code] ?? s.failure.code}: </b>}
            {s.error}
          </p>
        )}
        {s.value != null && (
          <p className="syn-br-step-value">
            Extracted <span className="mono">{s.value}</span>
          </p>
        )}
        {browserErrors.length > 0 && (
          <>
            <button className="link-btn syn-br-errors-toggle" onClick={() => setErrors((x) => !x)}>
              {errors ? '▾' : '▸'} {browserErrors.length} browser error{browserErrors.length === 1 ? '' : 's'}
            </button>
            {errors && (
              <ul className="syn-br-errors">
                {browserErrors.map((e, i) => (
                  <li key={i}>
                    <span className="syn-chip">{e.type === 'js' ? 'JS' : 'Network'}</span> <span className="mono syn-wrap">{e.name}</span> {e.description}
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </div>
      {s.status !== 'skipped' &&
        (shot ? (
          <button className="syn-br-thumb" onClick={onOpen} title="Open the screenshot">
            <img src={shot} alt={`Screenshot after step ${index}`} loading="lazy" />
          </button>
        ) : (
          <span className="syn-br-thumb empty" title="No screenshot kept (passing scheduled runs keep none)">
            <ImageOff size={16} />
          </span>
        ))}
    </li>
  );
}

/** A step's screenshot, large, with the previous and next steps' a key away. */
function Lightbox({ steps, shots, at, onMove, onClose }: { steps: BrowserStepResult[]; shots: (string | null)[]; at: number; onMove: (i: number) => void; onClose: () => void }) {
  const withShots = shots.map((s, i) => (s ? i : -1)).filter((i) => i >= 0);
  const pos = withShots.indexOf(at);
  const prev = withShots[pos - 1];
  const next = withShots[pos + 1];
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
      if (e.key === 'ArrowLeft' && prev !== undefined) onMove(prev);
      if (e.key === 'ArrowRight' && next !== undefined) onMove(next);
    };
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [prev, next, onMove, onClose]);
  return createPortal(
    <div className="syn-br-lightbox" role="dialog" aria-label="Screenshot" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="syn-br-lightbox-head">
        <span>
          <b>
            {at}. {steps[at]?.name}
          </b>{' '}
          <span className="muted mono">{steps[at]?.url}</span>
        </span>
        <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
          <X size={18} />
        </button>
      </div>
      <div className="syn-br-lightbox-body">
        <button className="icon-plain" title="Previous step (←)" disabled={prev === undefined} onClick={() => prev !== undefined && onMove(prev)}>
          <ChevronLeft size={28} />
        </button>
        {shots[at] && <img src={shots[at]!} alt={`Screenshot after step ${at}`} />}
        <button className="icon-plain" title="Next step (→)" disabled={next === undefined} onClick={() => next !== undefined && onMove(next)}>
          <ChevronRight size={28} />
        </button>
      </div>
    </div>,
    document.body,
  );
}
