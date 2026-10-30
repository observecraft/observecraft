import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useNavigate } from 'react-router-dom';
import { BookOpen, FileCode2, Plus } from 'lucide-react';
import { getProfile } from '../../../api/identity/client';
import { API_MODE } from '../../../api/http';
import { profileOverview } from '../../../api/profiles/client';
import { useTime } from '../../../components/shell/TimeContext';

const DOCS = 'https://github.com/observecraft/observecraft/blob/main/docs/sending-data.md#continuous-profiler';

/** Each question opens the view that answers it for the service. */
function questions(service: string): { label: string; to: string }[] {
  const q = `q=${encodeURIComponent(`service:${service}`)}`;
  return [
    { label: 'Which functions use the most CPU and memory in this service?', to: `/apm/profiles?${q}&metric=cpu-time` },
    { label: 'What insights are detected for this service?', to: `/apm/profiles/insights?service=${encodeURIComponent(service)}` },
    { label: 'Does this service show signs of a memory leak?', to: `/apm/profiles?${q}&view=metrics` },
    { label: 'Which functions use the most wall time in this service?', to: `/apm/profiles?${q}&metric=wall-time` },
  ];
}

/** Profiles → Overview: a greeting and the service to continue the investigation on. */
export function ProfilesOverview() {
  const navigate = useNavigate();
  const { range } = useTime();
  const me = useQuery({ queryKey: ['identity', 'profile'], queryFn: getProfile, enabled: API_MODE === 'live', staleTime: 60_000, retry: false });
  const services = useQuery({ queryKey: ['profiles', 'overview', range, ''], queryFn: () => profileOverview({ range, query: '' }) });
  const names = (services.data?.services ?? []).map((s) => s.service);
  const [picked, setPicked] = useState<string | null>(null);
  const service = picked ?? names[0] ?? '';
  const first = (me.data?.name ?? '').split(' ')[0];

  return (
    <div className="pf-overview">
      <h2>
        Hi{first ? `, ${first.toUpperCase()}` : ''} <span aria-hidden>👋</span>
      </h2>
      <div className="pf-overview-row">
        <span className="pf-overview-lead">Continue your investigation on:</span>
        <label className="pf-field">
          <span>Service</span>
          <select value={service} onChange={(e) => setPicked(e.target.value)} disabled={names.length === 0}>
            {names.length === 0 && <option value="">No profiled services</option>}
            {names.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <button className="pf-link-btn" disabled={!service} onClick={() => navigate(`/apm/profiles?q=${encodeURIComponent(`service:${service}`)}`)}>
          <Plus size={18} /> Filter
        </button>
      </div>
      {service ? (
        <div className="pf-overview-cards">
          {questions(service).map((q) => (
            <button key={q.label} className="pf-overview-card" onClick={() => navigate(q.to)}>
              <FileCode2 size={18} /> {q.label}
            </button>
          ))}
        </div>
      ) : (
        <a className="pf-overview-card" href={DOCS} target="_blank" rel="noreferrer">
          <BookOpen size={18} /> How do I enable Continuous Profiler for my service?
        </a>
      )}
    </div>
  );
}
