// Mirrors the Rust structs in summary/workflows/models.rs + neohive settings (serde camelCase).

export type WorkflowRunStatus =
  | 'queued'
  | 'running'
  | 'completed'
  | 'error'
  | 'cancelled';

export type NeoHiveRunStatus = 'none' | 'pushed' | 'partial' | 'failed';

export interface NeoHiveExportConfig {
  enabled: boolean;
  autoExport: boolean;
  /** section title -> memory type (e.g. { "Key Decisions": "decision" }) */
  sectionTypeOverrides: Record<string, string>;
  defaultType: string; // "narrative"
  importance: number;  // 1-10, default 6
}

export interface Workflow {
  id: string;
  name: string;
  description?: string | null;
  templateId: string;
  customPrompt?: string | null;
  provider: string;
  model: string;
  maxTokens?: number | null;
  temperature?: number | null;
  topP?: number | null;
  /** Raw JSON string of NeoHiveExportConfig as stored; parse if needed. */
  neohiveExport?: string | null;
  createdAt: string;
  updatedAt: string;
}

/** Payload for api_save_workflow (create if id omitted). */
export interface WorkflowInput {
  id?: string;
  name: string;
  description?: string | null;
  templateId: string;
  customPrompt?: string | null;
  provider: string;
  model: string;
  maxTokens?: number | null;
  temperature?: number | null;
  topP?: number | null;
  neohiveExport?: NeoHiveExportConfig | null;
}

export interface ParsedSection {
  title: string;
  content: string;
}

export interface WorkflowRun {
  id: string;
  workflowId?: string | null;
  workflowName: string;
  meetingId: string;
  status: WorkflowRunStatus;
  resultMarkdown?: string | null;
  /** JSON string: ParsedSection[] */
  resultSections?: string | null;
  error?: string | null;
  neohiveStatus: NeoHiveRunStatus;
  createdAt: string;
  updatedAt: string;
}

/** Cloudflare Access service-token connection settings (from api_get_neohive_config). */
export interface NeoHiveSettings {
  endpoint: string | null;
  accessClientId: string | null;
  accessClientSecret: string | null;
  enabled: boolean;
}

export interface ExportResult {
  pushed: number;
  failed: number;
}

export const DEFAULT_NEOHIVE_EXPORT: NeoHiveExportConfig = {
  enabled: false,
  autoExport: false,
  sectionTypeOverrides: {},
  defaultType: 'narrative',
  importance: 6,
};
