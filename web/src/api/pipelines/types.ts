// Log pipeline processors, in Datadog's pipeline format (what the server stores and runs; see
// server/crates/oc-api/src/pipelines). The generated schema types processors as plain objects.
import type { components } from '../generated/schema';
import type { JsonValue } from '../types';

type S = components['schemas'];

interface Base {
  name?: string;
  enabled?: boolean;
}

export interface GrokParser extends Base {
  type: 'grok-parser';
  /** Attribute parsed (default `message`). */
  source?: string;
  samples?: string[];
  grok: { supportRules?: string; matchRules?: string };
}

export type ReservedRemapperType = 'status-remapper' | 'date-remapper' | 'service-remapper' | 'message-remapper' | 'trace-id-remapper';

export interface ReservedRemapper extends Base {
  type: ReservedRemapperType;
  sources: string[];
}

export interface AttributeRemapper extends Base {
  type: 'attribute-remapper';
  sources: string[];
  sourceType?: 'attribute' | 'tag';
  target: string;
  targetType?: 'attribute' | 'tag';
  targetFormat?: 'auto' | 'string' | 'integer' | 'double';
  preserveSource?: boolean;
  overrideOnConflict?: boolean;
}

export interface CategoryProcessor extends Base {
  type: 'category-processor';
  target: string;
  categories: { filter: { query: string }; name: string }[];
}

export interface StringBuilder extends Base {
  type: 'string-builder-processor';
  template: string;
  target: string;
  replaceMissing?: boolean;
}

export interface LookupProcessor extends Base {
  type: 'lookup-processor';
  source: string;
  target: string;
  /** `key,value` lines. */
  lookupTable: string[];
  defaultLookup?: string;
}

export interface ArithmeticProcessor extends Base {
  type: 'arithmetic-processor';
  expression: string;
  target: string;
  replaceMissing?: boolean;
}

export interface UrlParser extends Base {
  type: 'url-parser';
  sources: string[];
  target: string;
}

export interface NestedPipeline extends Base {
  type: 'pipeline';
  filter: { query: string };
  processors: Processor[];
}

export type Processor = GrokParser | ReservedRemapper | AttributeRemapper | CategoryProcessor | StringBuilder | LookupProcessor | ArithmeticProcessor | UrlParser | NestedPipeline;
export type ProcessorType = Processor['type'];

/** The processor types the engine runs, with their labels. Others (from imported pipelines) are kept but skipped. */
export const PROCESSOR_TYPES: { type: ProcessorType; label: string; hint: string }[] = [
  { type: 'grok-parser', label: 'Grok Parser', hint: 'Extract attributes from text with grok rules' },
  { type: 'status-remapper', label: 'Status Remapper', hint: 'Set the log status from an attribute' },
  { type: 'date-remapper', label: 'Date Remapper', hint: 'Set the log date from an attribute' },
  { type: 'service-remapper', label: 'Service Remapper', hint: 'Set the service from an attribute' },
  { type: 'message-remapper', label: 'Message Remapper', hint: 'Set the message from an attribute' },
  { type: 'trace-id-remapper', label: 'Trace ID Remapper', hint: 'Link the log to a trace' },
  { type: 'attribute-remapper', label: 'Remapper', hint: 'Rename or copy attributes and tags' },
  { type: 'category-processor', label: 'Category Processor', hint: 'Set an attribute from the first matching search' },
  { type: 'string-builder-processor', label: 'String Builder', hint: 'Build an attribute from a template' },
  { type: 'lookup-processor', label: 'Lookup Processor', hint: 'Map values through a table' },
  { type: 'arithmetic-processor', label: 'Arithmetic Processor', hint: 'Compute a number from attributes' },
  { type: 'url-parser', label: 'URL Parser', hint: 'Split a URL into its parts' },
  { type: 'pipeline', label: 'Nested Pipeline', hint: 'Processors for a subset of the logs' },
];

export const isSupported = (type: string): type is ProcessorType => PROCESSOR_TYPES.some((t) => t.type === type);
export const processorLabel = (type: string) => PROCESSOR_TYPES.find((t) => t.type === type)?.label ?? type;

export type LogPipeline = Omit<S['LogPipeline'], 'processors'> & { processors: Processor[] };
export type LogPipelineInput = { name: string; filter: string; enabled: boolean; processors: Processor[] };
export type LibraryPipeline = Omit<S['LibraryPipeline'], 'processors'> & { processors: Processor[] };

/** A log for the test endpoint (S['SampleLog'] with its attributes typed). */
export interface SampleLog {
  message: string;
  status: string;
  service: string;
  host: string;
  source: string;
  tags: Record<string, string>;
  attributes: Record<string, JsonValue>;
  date: string | null;
  traceId: string;
}
export type PipelineTestRequest = { filter?: string | null; processors: Processor[]; log: SampleLog };
export interface PipelineTestResult {
  matched: boolean;
  log: SampleLog;
  errors: string[];
}
export type GrokTestRequest = { supportRules: string; matchRules: string; samples: string[] };
export interface GrokSampleResult {
  matched: boolean;
  rule: string | null;
  attributes: Record<string, JsonValue>;
}
export interface GrokTestResult {
  error: string | null;
  samples: GrokSampleResult[];
}
