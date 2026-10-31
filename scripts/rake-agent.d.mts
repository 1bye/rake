/** Native development automation; actions acknowledge admission, waits observe rendering. */
export interface Query {
  nodeId?: number; stableId?: string; id?: string; role?: string; name?: string;
  text?: string; textContains?: string; textContent?: string; textContentContains?: string; enabled?: boolean; visible?: boolean;
  bounds?: Partial<Bounds> & {tolerance?: number};
}
export interface Bounds {x: number; y: number; width: number; height: number}
export interface Revision {capture: string; metadata: string | null}
export interface RustSource {file: string; line: number; column: number; url?: string}
export interface Element {
  nodeId: number; parentId: number | null; stableId: string | null; id: string | null;
  type: string; role: string | null; name: string | null; text: string | null;
  nameTruncated: boolean; textTruncated: boolean; textContent: string; textContentTruncated: boolean; textContentComplete: boolean; bounds: Bounds | null;
  visibleBounds: Bounds | null; visible: boolean | null; enabled: boolean | null;
  focused: boolean | null; states: unknown; source: RustSource | null;
  metadataTruncated: boolean; styleEditable: boolean; textEditable: boolean;
}
export interface ElementReference extends Element {
  runtimeId: string; windowId: number; revision: Revision; frameSequence: string;
}
export interface Snapshot {
  runtimeId: string; windowId: number; revision: Revision; frameSequence: string;
  elements: Element[]; totalMatches: number; offset: number; hasMore: boolean;
  omitted: number; coverage: unknown; rootNodeId: number | null; queryComplete: boolean;
}
export interface NativeWindow {
  window: number; native_id: number; generation: string; frame: Bounds; content: Bounds;
  occluded: boolean; translucent: boolean;
}
export interface Capabilities {
  version: number; runtimeId: string; platform: string; errorContractVersion: number;
  agentQuery: {filters: string[]; snapshotOptions: string[]; textContent: string};
  methods: Record<string, {supported: boolean}>;
  sourceRead: {supported: boolean; transport: string};
  nativeWindowsAvailable: boolean; nativeError: string | null;
  limits: Record<string, number>; semantics: Record<string, string>;
}
export interface Wait {
  waitId: string; runtimeId: string; windowId: number;
  status: 'pending' | 'satisfied' | 'timed_out' | 'cancelled' | 'invalidated';
  remainingMs: number; observation: Snapshot | null; error: string | null;
}
export interface SourceSnippet {file: string; line: number; column: number; startLine: number; text: string; truncated?: boolean}
export interface CaptureMetadata {
  window: NativeWindow; format: 'png' | 'jpeg'; image: {width: number; height: number; kind: string; capturedAt?: number};
  scaleX: number; scaleY: number; inspectionGeneration: string; sampling: string;
}
export interface DiagnosticRecord {sequence: number; elapsed_us: number; level: string; kind: string; target: string; fields: Record<string, unknown>}
export interface DiagnosticEvidence {
  logs: DiagnosticRecord[]; networkErrors: DiagnosticRecord[]; omittedRetained: number;
  observedThrough: number | null; dropped: number; evicted: number; outputDropped: number; scope: string;
}
export interface EvidenceSection<T> {
  status: 'ok' | 'partial' | 'error' | 'truncated' | 'omitted'; data?: T;
  error?: {code: number; message: string; data?: Record<string, unknown>}; reason?: string; budgetBytes?: number;
}
export interface Evidence {
  runtimeId: string; windowId: number; frameSequence: string; revision: Revision;
  startedAtUnixMs: number; durationMs: number; complete: boolean; sampling: string;
  sections: {
    tree: EvidenceSection<Snapshot>; selected: EvidenceSection<Element>; source: EvidenceSection<SourceSnippet>;
    diagnostics: EvidenceSection<DiagnosticEvidence>; screenshot: EvidenceSection<{data: string; rake: CaptureMetadata}>;
  };
}
export interface Actionability {
  runtimeId: string; windowId: number; frameSequence: string; revision: Revision;
  element: Element; canDispatch: boolean; point: {x: number; y: number} | null;
  blockers: {code: string; message: string}[]; notes: {code: string; message: string}[];
  occlusion: {status: 'unknown'; scope: string; capturedHit?: {nodeId: number | null; id: string | null; type: string; relation: 'self' | 'descendant' | 'other'}; error?: string};
}
export class AgentError extends Error {
  constructor(category: string, message: string, options?: {code?: number; recovery?: string; outcome?: string; data?: Record<string, unknown>; cause?: unknown});
  code: number; category: string; recovery: string; outcome: string; data: Record<string, unknown>;
}
export class ActionabilityError extends AgentError {code: number; report: Actionability}
export class ProtocolError extends AgentError {code: number; method: string}
export class RakeAgent {
  private constructor();
  static connect(endpoint: string, options?: {targetId?: string; windowId?: number}): Promise<RakeAgent>;
  readonly capabilities: Capabilities; readonly runtimeId: string; readonly windowId?: number;
  /** Raw CDP; unknown transport outcomes are never automatically retried. */
  call<T = unknown>(method: string, params?: Record<string, unknown>, timeoutMs?: number): Promise<T>;
  windows(): Promise<NativeWindow[]>;
  selectWindow(windowId: number): Promise<NativeWindow>;
  snapshot(query?: Query, options?: {limit?: number; offset?: number; expectedRevision?: Revision; rootNodeId?: number}): Promise<Snapshot>;
  subtree(reference: ElementReference, query?: Query, options?: {limit?: number; offset?: number; expectedRevision?: Revision}): Promise<Snapshot>;
  /** Rejects missing and ambiguous matches. */
  find(query: Query): Promise<ElementReference>;
  actionability(reference: ElementReference): Promise<Actionability>;
  hover(reference: ElementReference): Promise<{accepted: true; beforeFrame: string}>;
  click(reference: ElementReference): Promise<{accepted: true; beforeFrame: string}>;
  scroll(reference: ElementReference, deltaY: number, deltaX?: number): Promise<{accepted: true; beforeFrame: string}>;
  drag(reference: ElementReference, dx: number, dy: number): Promise<{accepted: true; beforeFrame: string}>;
  key(key: string, modifiers?: number): Promise<{accepted: true}>;
  typeText(text: string): Promise<{accepted: true}>;
  waitFor(query: Query, options?: {state?: 'present' | 'absent'; timeoutMs?: number; afterFrame?: string; rootNodeId?: number; signal?: AbortSignal}): Promise<Wait>;
  evidence(options?: {query?: Query; limit?: number; screenshot?: boolean; diagnosticsAfter?: number; rootNodeId?: number}): Promise<Evidence>;
  screenshot(options?: {format?: 'png' | 'jpeg'; quality?: number}): Promise<{bytes: Uint8Array; metadata: CaptureMetadata}>;
  source(reference: ElementReference, radius?: number): Promise<SourceSnippet>;
  close(): void;
}
export function cliMain(args: string[]): Promise<void>;
export function cli(args: string[]): Promise<void>;
