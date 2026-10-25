import { useState } from 'react';
import { Search, X } from 'lucide-react';
import type { WidgetType } from '../../../api/dashboards/types';
import { WIDGET_LABEL, WIDGET_SECTIONS, WidgetIllustration } from './WidgetTiles';

const NEW: WidgetType[] = ['table', 'change'];

/** Right-hand "Add Widgets" tray. Clicking a tile opens the editor for a new widget of that type. */
export function AddWidgetsDrawer({ onPick, onClose }: { onPick: (t: WidgetType) => void; onClose: () => void }) {
  const [tab, setTab] = useState<'widgets' | 'powerpacks' | 'apps'>('widgets');
  const [search, setSearch] = useState('');
  const q = search.trim().toLowerCase();
  return (
    <aside className="aw-drawer" aria-label="Add widgets">
      <header className="aw-tabs">
        {(['widgets', 'powerpacks', 'apps'] as const).map((t) => (
          <button key={t} className={tab === t ? 'active' : ''} onClick={() => setTab(t)}>
            {t === 'widgets' ? 'Widgets' : t === 'powerpacks' ? 'Powerpacks' : 'Apps'}
          </button>
        ))}
        <button className="icon-plain aw-close" onClick={onClose} title="Close">
          <X size={18} />
        </button>
      </header>
      {tab === 'widgets' ? (
        <>
          <label className="aw-search">
            <Search size={15} />
            <input autoFocus placeholder="Search widgets" value={search} onChange={(e) => setSearch(e.target.value)} />
          </label>
          <div className="aw-scroll">
            {WIDGET_SECTIONS.map((s) => {
              const types = s.types.filter((t) => !q || WIDGET_LABEL[t].toLowerCase().includes(q));
              if (!types.length) return null;
              return (
                <section key={s.title}>
                  <h4>{s.title}</h4>
                  <div className="aw-grid">
                    {types.map((t) => (
                      <button key={t} className="aw-tile" onClick={() => onPick(t)}>
                        {NEW.includes(t) && <span className="aw-new badge-new">NEW</span>}
                        <span className="aw-art">
                          <WidgetIllustration type={t} />
                        </span>
                        <span className="aw-label">{WIDGET_LABEL[t]}</span>
                      </button>
                    ))}
                  </div>
                </section>
              );
            })}
          </div>
        </>
      ) : (
        <div className="aw-empty">
          {tab === 'powerpacks' ? 'Powerpacks (reusable widget groups) will appear here once your team saves one.' : 'Apps built on the ObserveCraft app framework will appear here.'}
        </div>
      )}
    </aside>
  );
}
