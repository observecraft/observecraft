// Kubernetes → Autoscaling: Summary, Cluster Scaling, Workload Scaling and Setup, with the org's
// prices (Settings) that idle costs and savings are computed with.
import { useState } from 'react';
import { NavLink, Outlet, useOutletContext } from 'react-router-dom';
import { Settings, X } from 'lucide-react';
import type { AutoscalingOverview, AutoscalingSettings } from '../../../api/kubernetes/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { OrchestrationHeader } from '../OrchestrationHeader';
import { useAutoscaling, useAutoscalingMutations } from './queries';
import '../../infrastructure/infrastructure.css';
import '../../settings/settings.css';
import '../../integrations/integrations.css';
import '../kubernetes.css';
import './autoscaling.css';

const TABS = [
  { to: '/kubernetes/autoscaling', label: 'Summary', end: true },
  { to: '/kubernetes/autoscaling/clusters', label: 'Cluster Scaling' },
  { to: '/kubernetes/autoscaling/workloads', label: 'Workload Scaling' },
  { to: '/kubernetes/autoscaling/setup', label: 'Setup' },
];

/** The overview, for the pages under the layout. */
export const useOverview = () => useOutletContext<AutoscalingOverview>();

export function AutoscalingLayout() {
  const overview = useAutoscaling();
  const [settingsOpen, setSettingsOpen] = useState(false);
  return (
    <div className="hosts-page ox-page as-page">
      <OrchestrationHeader product="kubernetes" active="Autoscaling" />
      <div className="ox-subbar as-subbar">
        <nav className="as-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `as-tab${isActive ? ' active' : ''}`}>
              {t.label}
            </NavLink>
          ))}
        </nav>
        <button className="btn" onClick={() => setSettingsOpen(true)} disabled={!overview.data}>
          <Settings size={15} /> Settings
        </button>
      </div>
      {overview.isPending ? <Loading label="Loading autoscaling…" /> : overview.isError ? <ErrorState error={overview.error} onRetry={() => overview.refetch()} /> : <Outlet context={overview.data} />}
      {settingsOpen && overview.data && <SettingsModal settings={overview.data.settings} onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}

function SettingsModal({ settings, onClose }: { settings: AutoscalingSettings; onClose: () => void }) {
  const [cpu, setCpu] = useState(String(settings.cpuHourPrice));
  const [memory, setMemory] = useState(String(settings.memoryGibHourPrice));
  const { settings: save } = useAutoscalingMutations();
  const valid = [cpu, memory].every((v) => v.trim() !== '' && Number.isFinite(Number(v)) && Number(v) >= 0);
  return (
    <Modal onClose={onClose} label="Autoscaling settings" width={560} height="auto">
      <form
        className="small-modal as-settings"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({ cpuHourPrice: Number(cpu), memoryGibHourPrice: Number(memory) }, { onSuccess: onClose });
        }}
      >
        <div className="modal-title-row">
          <h2>Settings</h2>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted">
          Idle costs and projected savings price the requested CPU and memory with these rates, over months of 730 hours. Use your cloud provider’s on-demand rates per vCPU and GiB.
        </p>
        <label className="form-label">
          USD per vCPU-hour
          <input type="number" min={0} step="0.0001" value={cpu} onChange={(e) => setCpu(e.target.value)} />
        </label>
        <label className="form-label">
          USD per GiB-hour of memory
          <input type="number" min={0} step="0.0001" value={memory} onChange={(e) => setMemory(e.target.value)} />
        </label>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="as-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!valid || save.isPending}>
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}
