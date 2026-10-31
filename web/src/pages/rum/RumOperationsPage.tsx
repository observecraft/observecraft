import { Workflow } from 'lucide-react';
import { useSelectedApplication } from './common';
import { useRumApplications } from './queries';
import { SelectApplicationHero } from './RumLayout';

/** RUM → Operations: the user journeys an application tracks, once one is selected. */
export function RumOperationsPage() {
  const [selected] = useSelectedApplication();
  const apps = useRumApplications();
  const app = apps.data?.find((a) => a.id === selected);
  if (!app) return <SelectApplicationHero />;
  return (
    <div className="rum-page">
      <section className="syn-unavailable">
        <Workflow size={34} strokeWidth={1.4} />
        <h2>Operations aren&rsquo;t available yet</h2>
        <p className="muted">
          Operations follow the critical journeys of {app.name} (sign-in, checkout, …) from start to end with their success rate. ObserveCraft doesn&rsquo;t track them yet; the application&rsquo;s
          sessions, views and errors are in Summary and Explore.
        </p>
      </section>
    </div>
  );
}
