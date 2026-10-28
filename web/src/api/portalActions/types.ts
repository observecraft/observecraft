import type { components } from '../generated/schema';

type S = components['schemas'];
export type ActionInput = S['ActionInput'];
export type ActionInputKind = S['ActionInputKind'];
export type ActionHeader = S['ActionHeader'];
export type ActionHttpRequest = S['ActionHttpRequest'];
export type ActionStep = S['ActionStep'];
export type ActionStepKind = S['ActionStepKind'];
export type PortalActionCategory = S['PortalActionCategory'];
export type PortalAction = S['PortalAction'];

/** Input values of a run, by input key. */
export type ActionValues = Record<string, string | number | boolean>;

/** What the editor sends: secrets by name, a value sets one, null removes it, names left out keep theirs. */
export type PortalActionInput = Omit<S['PortalActionInput'], 'secrets'> & { secrets?: Record<string, string | null> };

export type PortalActionRun = Omit<S['PortalActionRun'], 'inputs'> & { inputs: ActionValues };
