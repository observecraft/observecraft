import type { components } from '../generated/schema';

type S = components['schemas'];

export type AppComponentKind = S['AppComponentKind'];

/** A component with every field present (the server fills the defaults); see routes/apps.rs. */
export interface AppComponent {
  /** `[a-z0-9_]`, unique in the app; templates read inputs as `{{ components.<id>.value }}`. */
  id: string;
  kind: AppComponentKind;
  label: string;
  /** Heading and text: the content. */
  text: string;
  /** Inputs: pre-filled value and placeholder; select: options. */
  defaultValue: string;
  placeholder: string;
  options: string[];
  /** Button: the workflow it runs and a template per workflow input key. */
  workflowId: string;
  inputs: Record<string, string>;
  /** Table: the button whose latest run it shows, the list's path in the run data and its columns. */
  source: string;
  path: string;
  columns: string[];
  /** Metric: query, aggregation (avg, sum, min, max, last) and unit. */
  query: string;
  aggregation: string;
  unit: string;
  /** Columns of the 12-column grid (3, 4, 6, 8 or 12). */
  width: number;
}

export interface AppSpec {
  components: AppComponent[];
}

export type AutomationApp = Omit<S['AutomationApp'], 'spec'> & { spec: AppSpec };
export type AutomationAppInput = Omit<S['AutomationAppInput'], 'spec'> & { spec: AppSpec };
export type AutomationAppRun = S['AutomationAppRun'];

/** The app's input values, by component id. */
export type AppValues = Record<string, string | number | boolean>;
