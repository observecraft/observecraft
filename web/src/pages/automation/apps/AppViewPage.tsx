import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { ArrowLeft, LayoutGrid, Pencil } from 'lucide-react';
import type { AppComponent, AppValues } from '../../../api/apps/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { initialValues } from './appLogic';
import { AppRenderer, type ButtonRuns } from './AppRenderer';
import { useApp, useRunAppButton } from './queries';
import './apps.css';

/** A running app: its inputs, buttons that start workflow runs, and what those runs return. */
export function AppViewPage() {
  const { id } = useParams();
  const app = useApp(id);
  const run = useRunAppButton(id ?? '');
  const [values, setValues] = useState<AppValues>({});
  const [runs, setRuns] = useState<ButtonRuns>({});
  const [pressing, setPressing] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (app.data) setValues(initialValues(app.data.spec));
  }, [app.data]);

  if (app.isLoading) return <Loading />;
  if (app.isError || !app.data) return <ErrorState error={app.error} onRetry={() => app.refetch()} />;
  const a = app.data;

  const press = (button: AppComponent) => {
    setError(null);
    setPressing(button.id);
    run.mutate(
      { component: button.id, values },
      {
        onSuccess: (r) => setRuns((prev) => ({ ...prev, [button.id]: r })),
        onError: (e) => setError(e instanceof Error ? e.message : 'The button could not run'),
        onSettled: () => setPressing(null),
      },
    );
  };

  return (
    <div className="ab-view">
      <header className="ab-view-header">
        <Link to="/automation/apps" className="ab-icon-btn" aria-label="All apps">
          <ArrowLeft size={16} />
        </Link>
        <span className="ab-app-icon">
          <LayoutGrid size={16} />
        </span>
        <div className="ab-view-title">
          <h1>{a.name}</h1>
          {a.description && <p>{a.description}</p>}
        </div>
        <span className={`ab-chip ${a.published ? 'ab-chip-published' : 'ab-chip-draft'}`}>{a.published ? 'Published' : 'Draft'}</span>
        <Link className="btn" to={`/automation/apps/${a.id}/edit`}>
          <Pencil size={13} /> Edit
        </Link>
      </header>
      {error && (
        <div className="ab-alert" role="alert">
          {error}
        </div>
      )}
      <main className="ab-view-body">
        <div className="ab-app-surface">
          <AppRenderer spec={a.spec} values={values} onValue={(k, v) => setValues((p) => ({ ...p, [k]: v }))} onPress={press} runs={runs} pressing={pressing} />
        </div>
      </main>
    </div>
  );
}
