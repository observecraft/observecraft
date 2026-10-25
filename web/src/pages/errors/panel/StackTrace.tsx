import { useRef, useState } from 'react';
import { ChevronsUpDown, Copy, Maximize2 } from 'lucide-react';
import type { ErrorSample, StackFrame } from '../../../api/errorTracking/types';

const HEAD = 3;
const TAIL = 2;

function Frame({ f }: { f: StackFrame }) {
  return (
    <div className={`st-frame${f.inApp ? ' in-app' : ''}`}>
      <span className="st-index">{f.index}</span>
      {f.address ? (
        <span className="st-body">
          {f.module === '[vdso]' ? (
            <>
              <span className="st-module">{f.module}</span>
              <span className="st-addr">{f.address}</span>
            </>
          ) : (
            <>
              {f.module} <span className="st-addr-inline">{f.address}</span>
            </>
          )}{' '}
          {f.symbol}
        </span>
      ) : (
        <span className="st-body">
          {f.module}.<strong>{f.symbol}</strong>
          {f.file && (
            <span className="st-file">
              {' '}
              ({f.file}
              {f.line ? `:${f.line}` : ''})
            </span>
          )}
        </span>
      )}
    </div>
  );
}

/** Pretty (frame list, middle collapsed) / Raw (plain text) stack trace with copy + fullscreen. */
export function StackTrace({ stack }: { stack: ErrorSample['stack'] }) {
  const [mode, setMode] = useState<'pretty' | 'raw'>('pretty');
  const [all, setAll] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const frames = stack.frames;
  const collapsible = !all && frames.length > HEAD + TAIL + 1;

  return (
    <div className="stack" ref={box}>
      <div className="stack-toolbar">
        <div className="btn-group">
          <button className={`btn btn-sm${mode === 'pretty' ? ' btn-selected' : ''}`} onClick={() => setMode('pretty')}>
            Pretty
          </button>
          <button className={`btn btn-sm${mode === 'raw' ? ' btn-selected' : ''}`} onClick={() => setMode('raw')}>
            Raw
          </button>
        </div>
        <span className="stack-tools">
          <button className="icon-plain" title="Copy stack trace" onClick={() => navigator.clipboard?.writeText(stack.raw)}>
            <Copy size={16} />
          </button>
          <button className="icon-plain" title="Full screen" onClick={() => box.current?.requestFullscreen?.()}>
            <Maximize2 size={16} />
          </button>
        </span>
      </div>
      {mode === 'raw' ? (
        <pre className="stack-raw mono">{stack.raw}</pre>
      ) : (
        <div className="stack-pretty mono">
          <div className="st-title">
            <strong>{stack.title.split(':')[0]}:</strong>
            {stack.title.slice(stack.title.indexOf(':') + 1)}
          </div>
          {(collapsible ? frames.slice(0, HEAD) : frames).map((f) => (
            <Frame key={f.index} f={f} />
          ))}
          {collapsible && (
            <>
              <button className="st-expand" onClick={() => setAll(true)}>
                <ChevronsUpDown size={15} /> Show all frames
              </button>
              {frames.slice(-TAIL).map((f) => (
                <Frame key={f.index} f={f} />
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
