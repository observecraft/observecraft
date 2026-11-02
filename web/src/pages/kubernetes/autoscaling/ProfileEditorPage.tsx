// Autoscaling → Profiles → Create profile: the scaling behavior of a DatadogPodAutoscalerClusterProfile
// on the left, its manifest (updated as the form changes) and how to deploy it on the right. Saving
// keeps it in ObserveCraft; the Cluster Agent applies it once it is in a cluster.
import { useMemo, useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ArrowLeftRight, ArrowUpDown, Check, ChevronRight, Layers } from 'lucide-react';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { OrchestrationHeader } from '../OrchestrationHeader';
import { DEFAULT_PROFILE, PROFILE_LABEL, formFromTemplate, profileManifest, profileTemplate, type Dimensions, type ProfileForm } from './manifests';
import { useAutoscaling, useAutoscalingMutations } from './queries';
import '../../infrastructure/infrastructure.css';
import '../../settings/settings.css';
import '../../integrations/integrations.css';
import '../kubernetes.css';
import './autoscaling.css';

const DIMENSIONS: { id: Dimensions; title: string; text: string; icon: ReactNode }[] = [
  { id: 'both', title: 'Multidimensional Scaling', text: 'Scale replicas horizontally on CPU and set CPU and memory requests vertically.', icon: <Layers size={16} /> },
  { id: 'vertical', title: 'Vertical only', text: 'Set CPU and memory requests from usage; replicas stay as they are.', icon: <ArrowUpDown size={16} /> },
  { id: 'horizontal', title: 'Horizontal only', text: 'Adjust replica counts on CPU utilization; requests stay as they are.', icon: <ArrowLeftRight size={16} /> },
];

const NAME = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$/;

export function ProfileEditorPage() {
  const { id } = useParams();
  const [params] = useSearchParams();
  const overview = useAutoscaling();
  const profiles = overview.data?.profiles ?? [];
  const existing = id ? profiles.find((p) => p.id === id) : undefined;
  const source = profiles.find((p) => p.name === params.get('from'));
  if (id && overview.isPending) return null;
  const key = existing?.name ?? source?.name ?? 'new';
  return (
    <Editor
      key={key}
      id={id}
      initialName={existing?.name ?? (source ? `${source.name}-copy` : 'my-autoscaling-profile')}
      initial={existing ? formFromTemplate(existing.template) : source ? formFromTemplate(source.template) : DEFAULT_PROFILE}
    />
  );
}

function Editor({ id, initialName, initial }: { id?: string; initialName: string; initial: ProfileForm }) {
  const navigate = useNavigate();
  const [form, setForm] = useState<ProfileForm>(initial);
  const [name, setName] = useState(initialName);
  const { create, update } = useAutoscalingMutations();
  const set = <K extends keyof ProfileForm>(k: K, v: ProfileForm[K]) => setForm((f) => ({ ...f, [k]: v }));
  const template = useMemo(() => profileTemplate(form), [form]);
  const nameOk = NAME.test(name) && name.length <= 253;
  const horizontal = form.dimensions !== 'vertical';
  const valid = nameOk && (!horizontal || (form.minReplicas >= 1 && form.maxReplicas >= form.minReplicas && form.cpuTarget >= 1 && form.cpuTarget <= 100));
  const save = () => {
    const input = { name, template };
    const done = { onSuccess: () => navigate('/kubernetes/autoscaling/workloads?view=profiles') };
    if (id) update.mutate({ id, input }, done);
    else create.mutate(input, done);
  };
  const error = (create.error ?? update.error) as Error | null;
  return (
    <div className="hosts-page ox-page as-page">
      <OrchestrationHeader product="kubernetes" active="Autoscaling" />
      <div className="product-title rt-crumb as-crumbs">
        <Link to="/kubernetes/autoscaling/workloads?view=profiles">Workload Scaling Profiles</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>{id ? 'Edit profile' : 'Create profile'}</span>
      </div>
      <div className="as-editor">
        <section className="as-editor-form">
          <h2>Scaling configuration</h2>
          <p className="muted">Configure the scaling behavior of the profile. The manifest updates as you make changes.</p>
          <Section title="Set scaling dimensions" hint="Enable scaling on one or both dimensions.">
            <div className="as-choices">
              {DIMENSIONS.map((d) => (
                <button key={d.id} type="button" className={`as-choice${form.dimensions === d.id ? ' selected' : ''}`} onClick={() => set('dimensions', d.id)}>
                  <span className="as-choice-title">
                    {form.dimensions === d.id ? <Check size={16} /> : d.icon} {d.title}
                  </span>
                  <span className="muted">{d.text}</span>
                </button>
              ))}
            </div>
          </Section>
          {horizontal && (
            <Section title="Horizontal CPU settings">
              <label className="form-label">
                CPU utilization target (%)
                <span className="muted">The autoscaler adds or removes replicas to keep CPU usage at this share of the requests.</span>
                <span className="as-slider">
                  <input type="range" min={10} max={100} value={form.cpuTarget} onChange={(e) => set('cpuTarget', Number(e.target.value))} />
                  <input type="number" min={1} max={100} value={form.cpuTarget} onChange={(e) => set('cpuTarget', Number(e.target.value))} />
                  <span>%</span>
                  <button type="button" className="link-btn" onClick={() => set('cpuTarget', DEFAULT_PROFILE.cpuTarget)}>
                    Use default
                  </button>
                </span>
              </label>
              <label className="form-label">
                Measure CPU utilization on
                <span className="muted">The pod (all its containers), or one container by name.</span>
                <span className="as-inline">
                  <select value={form.container ? 'container' : 'pod'} onChange={(e) => set('container', e.target.value === 'pod' ? '' : 'app')}>
                    <option value="pod">Pod</option>
                    <option value="container">Container</option>
                  </select>
                  {form.container !== '' && <input value={form.container} onChange={(e) => set('container', e.target.value)} placeholder="container name" />}
                </span>
              </label>
              <div className="form-label">
                Replica count range
                <span className="muted">Control the range of running application copies.</span>
                <span className="as-inline">
                  <label>
                    Minimum <input type="number" min={1} value={form.minReplicas} onChange={(e) => set('minReplicas', Number(e.target.value))} />
                  </label>
                  <label>
                    Maximum <input type="number" min={1} value={form.maxReplicas} onChange={(e) => set('maxReplicas', Number(e.target.value))} />
                  </label>
                </span>
              </div>
            </Section>
          )}
          {horizontal && (
            <Section title="Scaling speed" hint="How fast replicas are added and removed.">
              <Rule
                label="Scale up"
                percent={form.scaleUpPercent}
                period={form.scaleUpPeriodSeconds}
                window={form.scaleUpWindowSeconds}
                onChange={(p, s, w) => setForm((f) => ({ ...f, scaleUpPercent: p, scaleUpPeriodSeconds: s, scaleUpWindowSeconds: w }))}
              />
              <Rule
                label="Scale down"
                percent={form.scaleDownPercent}
                period={form.scaleDownPeriodSeconds}
                window={form.scaleDownWindowSeconds}
                onChange={(p, s, w) => setForm((f) => ({ ...f, scaleDownPercent: p, scaleDownPeriodSeconds: s, scaleDownWindowSeconds: w }))}
              />
            </Section>
          )}
          {form.dimensions !== 'horizontal' && (
            <Section title="Vertical settings">
              <p className="muted">
                Every container’s CPU and memory requests follow its usage. The Cluster Agent applies vertical values from Datadog’s Remote Configuration; with ObserveCraft, apply the recommended
                requests from Scaling Recommendations.
              </p>
            </Section>
          )}
        </section>
        <section className="as-editor-deploy">
          <h2>Deployment instructions</h2>
          <p className="muted">Deploy the cluster profile to each target cluster, then label the workloads you want to scale.</p>
          <div className="as-step">
            <span className="as-step-num">1</span>
            <div>
              <h3>Make the profile available in each target cluster</h3>
              <p className="muted">Applying the profile alone does not scale any workload.</p>
              <label className="form-label">
                Cluster profile name <span className="rt-required">*</span>
                <input value={name} onChange={(e) => setName(e.target.value.trim())} />
                {!nameOk && <span className="form-error">Lowercase letters, digits, ‘-’ and ‘.’.</span>}
              </label>
              <CodeBlock code={profileManifest(name || 'my-autoscaling-profile', template)} filename={`${name || 'profile'}.yaml`} />
            </div>
          </div>
          <div className="as-step">
            <span className="as-step-num">2</span>
            <div>
              <h3>Label the workloads to scale</h3>
              <p className="muted">The Cluster Agent creates a DatadogPodAutoscaler for every Deployment or StatefulSet with this label.</p>
              <CodeBlock code={`kubectl -n <namespace> label deployment <name> ${PROFILE_LABEL}=${name || 'my-autoscaling-profile'}`} filename="label.sh" />
            </div>
          </div>
          {error && <p className="form-error">{error.message}</p>}
          <div className="as-actions">
            <Link className="btn" to="/kubernetes/autoscaling/workloads?view=profiles">
              Cancel
            </Link>
            <button className="btn btn-primary" disabled={!valid || create.isPending || update.isPending} onClick={save}>
              {id ? 'Save profile' : 'Save to ObserveCraft'}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
}

function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <div className="as-section">
      <h3>{title}</h3>
      {hint && <p className="muted">{hint}</p>}
      {children}
    </div>
  );
}

function Rule({ label, percent, period, window, onChange }: { label: string; percent: number; period: number; window: number; onChange: (percent: number, period: number, window: number) => void }) {
  return (
    <div className="as-rule">
      <strong>{label}</strong>
      <label>
        at most <input type="number" min={1} max={1000} value={percent} onChange={(e) => onChange(Number(e.target.value), period, window)} /> % of the replicas
      </label>
      <label>
        every <input type="number" min={1} value={period} onChange={(e) => onChange(percent, Number(e.target.value), window)} /> s
      </label>
      <label>
        stabilization window <input type="number" min={0} value={window} onChange={(e) => onChange(percent, period, Number(e.target.value))} /> s
      </label>
    </div>
  );
}
