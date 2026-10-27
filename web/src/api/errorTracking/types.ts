// Error Tracking wire contract. Errors from logs, RUM and APM spans are fingerprinted
// server-side into Issues; the UI lists issues, drills into one, and browses its samples.
import type { Facet, JsonValue, QuerySuggestion, TimeRange } from '../types';

export type { QuerySuggestion };

export type IssueStatus = 'for_review' | 'reviewed' | 'ignored' | 'resolved';
export type IssueStateTab = IssueStatus | 'all';
export type Persona = 'all' | 'backend' | 'browser' | 'mobile';
export type DataSource = 'logs' | 'rum' | 'apm';
export type Platform = 'android' | 'ios' | 'java' | 'dotnet' | 'browser' | 'react-native' | 'node' | 'python' | 'go' | 'unknown';
export type SuspectedCause = 'code_exception' | 'failed_request' | 'lost_connection' | 'invalid_argument' | 'illegal_object_access' | 'unknown';
export type IssueSort = 'relevance' | 'count' | 'last_seen' | 'first_seen';

export interface User {
  id: string;
  name: string;
  email: string;
}

export interface Issue {
  id: string;
  errorType: string; // "java.lang.NullPointerException", "SIGSEGV"
  message: string;
  platform: Platform;
  persona: Exclude<Persona, 'all'>;
  source: DataSource;
  service: string;
  /** Mobile/browser app identity, e.g. { name: "shop-mobile-app", packageId: "com.example.shop" }. */
  application?: { name: string; packageId?: string };
  /** Top in-app frame, shown as "addAll in base/java.util.ArrayList.java". */
  location?: { function: string; file: string };
  firstSeen: number;
  lastSeen: number;
  firstSeenVersion?: string;
  lastSeenVersion?: string;
  /** Occurrences in the requested range, plus a 12-bucket sparkline over that range. */
  count: number;
  sparkline: number[];
  status: IssueStatus;
  assignee: User | null;
  team: string | null;
  isCrash: boolean;
  isRegression: boolean;
  suspectedCause: SuspectedCause | null;
}

export interface IssueSearchRequest {
  range: TimeRange;
  query: string;
  state: IssueStateTab;
  persona: Persona;
  source: DataSource | 'all';
  assignee: string | 'all' | 'unassigned'; // user id
  suspectedCause: SuspectedCause | 'all';
  team: string | 'all';
  /** Only issues with enough context (in-app frames) for an automated investigation. */
  investigationReady: boolean;
  sort: IssueSort;
  cursor?: string;
  limit: number;
}

export interface IssueSearchPage {
  issues: Issue[];
  nextCursor: string | null;
  /** Matching issue counts per state tab (ignores the `state` filter). */
  totals: Record<IssueStateTab, number>;
}

/** Facet counts span all state tabs. */
export type IssueFacetsRequest = Omit<IssueSearchRequest, 'state' | 'sort' | 'cursor' | 'limit'>;

export interface IssueFacets {
  facets: Facet[];
  totalAvailable: number;
}

export interface TagDistribution {
  key: string; // "@view.name", "version"
  value: string;
  share: number; // 0..1 of the issue's errors
}

export type ActivityEntry =
  | { kind: 'status'; at: number; by: User | null; from: IssueStatus; to: IssueStatus }
  | { kind: 'assign'; at: number; by: User | null; to: User | null }
  | { kind: 'comment'; at: number; by: User; text: string }
  | { kind: 'regression'; at: number; version?: string }
  | { kind: 'created'; at: number };

export interface IssueDetail extends Issue {
  histogram: { t: number; count: number }[];
  impact: { sessions: number; sessionShare: number; views: number; users: number } | null;
  tags: TagDistribution[];
  activity: ActivityEntry[];
}

export interface StackFrame {
  index: number;
  module: string; // "libhermes.so", "java.util.ArrayList"
  address?: string;
  symbol: string;
  file?: string;
  line?: number;
  inApp: boolean;
}

export interface TimelineEvent {
  at: number;
  type: 'crash' | 'error' | 'view' | 'action' | 'resource' | 'log';
  label: string; // "Crash", "View Load"
  message: string;
  /** Bold part of the message ("PreLoginScreen"). */
  emphasis?: string;
  isCurrent?: boolean;
}

export interface AttributeGroup {
  name: string; // "View", "Session", "Device"
  attributes: { label: string; value: string }[];
}

export interface ErrorSample {
  id: string;
  timestamp: number;
  tags: Record<string, string>;
  stack: { title: string; frames: StackFrame[]; raw: string };
  timeline: TimelineEvent[];
  attributeGroups: AttributeGroup[];
  json: Record<string, JsonValue>;
}

export interface ErrorSampleRequest {
  issueId: string;
  range: TimeRange;
  query: string;
  index: number; // position within `order`
  order: 'latest' | 'oldest';
}

export interface ErrorSamplePage {
  sample: ErrorSample | null;
  index: number;
  total: number;
}

export interface IssuePatch {
  status?: IssueStatus;
  assigneeId?: string | null;
  suspectedCause?: SuspectedCause | null;
}
