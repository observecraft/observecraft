import { useState } from 'react';
import { AtSign, Briefcase, ChevronRight, CirclePlus, Layers, ListChecks, PanelLeftClose, PanelLeft, Search, Settings, SlidersHorizontal } from 'lucide-react';
import { TimePicker } from '../../components/shell/PageHeader';

/**
 * Event Management → Triage Inbox: work items that correlators group from related events.
 * No correlators exist yet, so every view is empty.
 */
export function TriageInbox() {
  const [showProjects, setShowProjects] = useState(true);
  const [query, setQuery] = useState('');
  const views = ['Open', 'In Progress', 'Closed', 'Archived'];

  return (
    <div className="ev-inbox">
      {showProjects && (
        <aside className="ev-projects">
          <button className="ev-projects-toggle" onClick={() => setShowProjects(false)}>
            <PanelLeftClose size={16} /> Hide Projects
          </button>
          <button className="ev-proj-row">
            <Search size={16} /> Search
          </button>
          <button className="ev-proj-row">
            <AtSign size={16} /> Assigned To You <ChevronRight size={16} className="ev-proj-end" />
          </button>
          <div className="ev-proj-title">Projects</div>
          <button className="ev-proj-row">
            <Briefcase size={16} /> Event Management <ChevronRight size={16} className="ev-proj-end" />
          </button>
          <button className="ev-proj-row">
            <Briefcase size={16} /> Intelligent Correlation <Settings size={15} className="ev-proj-end muted" />
          </button>
          <button className="ev-proj-row ev-proj-sub active">
            <ListChecks size={16} /> All Work Items <span className="ev-proj-end">0</span>
          </button>
          {views.map((v) => (
            <button key={v} className="ev-proj-row ev-proj-sub2">
              {v} <span className="ev-proj-end">0</span>
            </button>
          ))}
          <button className="ev-proj-row ev-proj-sub ev-proj-add">
            <CirclePlus size={16} /> Add View
          </button>
        </aside>
      )}
      <div className="ev-inbox-main">
        <div className="ev-inbox-bar">
          {!showProjects && (
            <button className="btn" onClick={() => setShowProjects(true)} title="Show projects">
              <PanelLeft size={15} />
            </button>
          )}
          <button className="btn ev-inbox-filter" title="Filters">
            <SlidersHorizontal size={15} />
          </button>
          <label className="ev-inbox-search">
            <Search size={15} className="muted" />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search Correlation Work Items" />
          </label>
          <TimePicker compact />
        </div>
        <div className="ev-inbox-empty">
          <Search size={96} strokeWidth={1.5} className="ev-inbox-empty-icon" />
          <h3>No results found</h3>
          <p>
            You might want to expand your filtering criteria. <button className="link">Update time range selection.</button>
          </p>
        </div>
        <div className="ev-inbox-tip">
          <div>
            <h4>Common filters to check back on?</h4>
            <p>Save a customized view instead of hunting for the right work items every time.</p>
          </div>
          <div className="ev-inbox-card">
            <h5>
              <Layers size={15} /> Customized View
            </h5>
            <p>Save a customized view using filters that's relevant to you or your team</p>
          </div>
        </div>
      </div>
    </div>
  );
}
