# Meeting Workflows — Frontend Implementation Plan (Plan 2 of 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Depends on Plan 1 (backend) being implemented first** — every Tauri command consumed here is defined there.

**Goal:** Add the Next.js UI for meeting workflows: a Workflows manager in Settings (create/edit/delete recipes + NeoHive connection config), a "Run workflow" control on the meeting view, retained run-result cards shown alongside the summary, and a per-run "Send to NeoHive" action.

**Architecture:** Thin UI over Plan 1's Tauri commands. New TS types mirror the backend structs; two hooks (`useWorkflows`, `useWorkflowRuns`) own data + the 5-second poll (mirroring `SidebarProvider.startSummaryPolling`, but self-contained and keyed by run id). Reused building blocks: the provider/model picker pattern from `ModelSettingsModal.tsx`, `useTemplates` for the template dropdown, `useConfig()` for provider model options + API keys. Run controls and cards are self-contained components mounted in `SummaryPanel.tsx`, consuming only `meetingId` + the transcript text already in scope.

**Tech Stack:** Next.js (App Router), React, TypeScript, Tailwind, shadcn/ui primitives (`Select`, `Popover`, `Command`, `Button`, `Input`, `Switch`, `Tabs`), `sonner` toasts, `@tauri-apps/api/core` `invoke`.

**Scope:** Frontend only + one small backend-glue task (Task 1) to expose the NeoHive config commands the UI needs. Assumes Plan 1's commands exist: `api_list_workflows`, `api_save_workflow`, `api_delete_workflow`, `api_run_workflow`, `api_get_workflow_run`, `api_list_workflow_runs`, `api_cancel_workflow_run`, `api_export_run_to_neohive`.

## Global Constraints

- **No frontend test runner exists** and this plan does NOT add one (out of scope / not requested). Testable logic lives in Rust (Plan 1). Frontend task verification gates are: **`cd frontend && npx tsc --noEmit`** (type safety) + **`pnpm lint`** (next lint) + a **manual check** in a running dev app (`./clean_run.sh` from `frontend/`, dev URL http://localhost:3118, DevTools Cmd+Shift+I). Every task ends with typecheck + lint clean and a stated manual observation.
- **Tauri invoke:** `import { invoke } from '@tauri-apps/api/core';` (or `invoke as invokeTauri` to match neighbors). Args are camelCase; Tauri maps them to the backend's snake_case params. For `api_save_workflow`, pass `{ workflow: <WorkflowInput> }` where the nested object uses **camelCase** keys (`templateId`, `customPrompt`, `maxTokens`, `topP`, `neohiveExport`) — the backend `WorkflowInput` derives `#[serde(rename_all = "camelCase")]`.
- **Follow existing patterns:** hooks live in `frontend/src/hooks/meeting-details/`; settings sections mirror `SummaryModelSettings.tsx`; import `ModelConfig` from `@/components/ModelSettingsModal` (NOT from `@/types`); import shared types from `@/types` (the `index.ts` barrel — NOT `@/types/summary`).
- **Never render or log** the NeoHive token or provider API keys in plaintext (mask like `ModelSettingsModal` does with a show/hide toggle).
- **DRY / YAGNI / frequent commits.** Commit after each task's gates pass. Commit style: `feat(workflows): :sparkles: <desc>` (gitmoji conventional).

## File Structure

**Create:**
- `frontend/src/types/workflow.ts` — `Workflow`, `WorkflowRun`, `WorkflowInput`, `NeoHiveExportConfig`, `NeoHiveSettings`, status union.
- `frontend/src/hooks/meeting-details/useWorkflows.ts` — list/save/delete + local state.
- `frontend/src/hooks/meeting-details/useWorkflowRuns.ts` — list runs + run + poll + cancel + export.
- `frontend/src/components/workflows/ModelPicker.tsx` — provider + model sub-picker (reused in the editor).
- `frontend/src/components/workflows/WorkflowEditor.tsx` — create/edit form.
- `frontend/src/components/workflows/WorkflowsSettings.tsx` — manager (list + editor + NeoHive connection card).
- `frontend/src/components/MeetingDetails/WorkflowRunCard.tsx` — one run's result card (copy + Send to NeoHive).
- `frontend/src/components/MeetingDetails/WorkflowRunSection.tsx` — "Run workflow ▾" control + stacked cards; self-contained via hooks.

**Modify:**
- `frontend/src-tauri/src/summary/workflows/commands.rs` + `lib.rs` — add `api_get_neohive_config` / `api_save_neohive_config` (Task 1).
- `frontend/src/app/settings/page.tsx` — add a "Workflows" tab.
- `frontend/src/components/MeetingDetails/SummaryPanel.tsx` — mount `WorkflowRunSection`.

---

## Phase 1 — Backend glue: NeoHive config commands

### Task 1: `api_get_neohive_config` / `api_save_neohive_config`

**Files:**
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `SettingsRepository::{get_neohive_config, save_neohive_config}` + `NeoHiveSettings` (Plan 1 Task 9).
- Produces: `api_get_neohive_config() -> NeoHiveConfigResponse { endpoint, apiKey, enabled }`, `api_save_neohive_config(endpoint, api_key, enabled) -> ()`.

- [ ] **Step 1: Add the response struct + commands to `commands.rs`**

```rust
use crate::database::repositories::setting::SettingsRepository;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeoHiveConfigResponse {
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub enabled: bool,
}

#[tauri::command]
pub async fn api_get_neohive_config(
    state: tauri::State<'_, AppState>,
) -> Result<NeoHiveConfigResponse, String> {
    let cfg = SettingsRepository::get_neohive_config(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(NeoHiveConfigResponse { endpoint: cfg.endpoint, api_key: cfg.api_key, enabled: cfg.enabled })
}

#[tauri::command]
pub async fn api_save_neohive_config(
    state: tauri::State<'_, AppState>,
    endpoint: Option<String>,
    api_key: Option<String>,
    enabled: bool,
) -> Result<(), String> {
    SettingsRepository::save_neohive_config(
        state.db_manager.pool(),
        endpoint.as_deref(),
        api_key.as_deref(),
        enabled,
    )
    .await
    .map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register both in `lib.rs`** (under the Workflow commands block)

```rust
            summary::workflows::commands::api_get_neohive_config,
            summary::workflows::commands::api_save_neohive_config,
```

- [ ] **Step 3: Verify build**

Run: `cd frontend/src-tauri && cargo check`
Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(workflows): :sparkles: expose NeoHive config get/save commands"
```

---

## Phase 2 — Types & data hooks

### Task 2: TypeScript types

**Files:**
- Create: `frontend/src/types/workflow.ts`

**Interfaces:**
- Produces: `Workflow`, `WorkflowRun`, `WorkflowInput`, `NeoHiveExportConfig`, `NeoHiveSettings`, `WorkflowRunStatus`, `ParsedSection` — consumed by all later tasks.

- [ ] **Step 1: Write the file**

```ts
// Mirrors the Rust structs in summary/workflows/models.rs (serde camelCase).

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

export interface NeoHiveSettings {
  endpoint: string | null;
  apiKey: string | null;
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
```

- [ ] **Step 2: Verify typecheck**

Run: `cd frontend && npx tsc --noEmit`
Expected: no new errors (file has no runtime deps).

- [ ] **Step 3: Commit**

```bash
git add frontend/src/types/workflow.ts
git commit -m "feat(workflows): :sparkles: add workflow TypeScript types"
```

---

### Task 3: `useWorkflows` hook

**Files:**
- Create: `frontend/src/hooks/meeting-details/useWorkflows.ts`

**Interfaces:**
- Consumes: `api_list_workflows`, `api_save_workflow`, `api_delete_workflow`; types from `@/types/workflow`.
- Produces: `useWorkflows()` returning `{ workflows, isLoading, refresh, saveWorkflow, deleteWorkflow }`.

- [ ] **Step 1: Write the hook**

```ts
import { useCallback, useEffect, useState } from 'react';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Workflow, WorkflowInput } from '@/types/workflow';

export function useWorkflows() {
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [isLoading, setIsLoading] = useState(false);

  const refresh = useCallback(async () => {
    setIsLoading(true);
    try {
      const list = await invokeTauri<Workflow[]>('api_list_workflows');
      setWorkflows(list);
    } catch (err) {
      console.error('Failed to list workflows:', err);
      toast.error('Failed to load workflows');
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const saveWorkflow = useCallback(async (input: WorkflowInput): Promise<Workflow | null> => {
    try {
      const saved = await invokeTauri<Workflow>('api_save_workflow', { workflow: input });
      toast.success(input.id ? 'Workflow updated' : 'Workflow created');
      await refresh();
      return saved;
    } catch (err) {
      console.error('Failed to save workflow:', err);
      toast.error(`Failed to save workflow: ${err instanceof Error ? err.message : String(err)}`);
      return null;
    }
  }, [refresh]);

  const deleteWorkflow = useCallback(async (workflowId: string): Promise<boolean> => {
    try {
      await invokeTauri<boolean>('api_delete_workflow', { workflowId });
      toast.success('Workflow deleted');
      await refresh();
      return true;
    } catch (err) {
      console.error('Failed to delete workflow:', err);
      toast.error('Failed to delete workflow');
      return false;
    }
  }, [refresh]);

  return { workflows, isLoading, refresh, saveWorkflow, deleteWorkflow };
}
```

- [ ] **Step 2: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/hooks/meeting-details/useWorkflows.ts
git commit -m "feat(workflows): :sparkles: add useWorkflows data hook"
```

---

### Task 4: `useWorkflowRuns` hook (list + run + poll + cancel + export)

**Files:**
- Create: `frontend/src/hooks/meeting-details/useWorkflowRuns.ts`

**Interfaces:**
- Consumes: `api_list_workflow_runs`, `api_run_workflow`, `api_get_workflow_run`, `api_cancel_workflow_run`, `api_export_run_to_neohive`.
- Produces: `useWorkflowRuns(meetingId)` → `{ runs, isLoading, refresh, runWorkflow, cancelRun, exportRun, activeRunId }`.

**Polling contract (mirrors `SidebarProvider.startSummaryPolling`):** after `api_run_workflow` returns `{ runId }`, poll `api_get_workflow_run({ runId })` every 5000ms; stop on terminal status (`completed`/`error`/`cancelled`) or after 200 polls; refresh the run list each tick.

- [ ] **Step 1: Write the hook**

```ts
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ExportResult, WorkflowRun } from '@/types/workflow';

const POLL_MS = 5000;
const MAX_POLLS = 200;
const TERMINAL: WorkflowRun['status'][] = ['completed', 'error', 'cancelled'];

export function useWorkflowRuns(meetingId: string | undefined) {
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const refresh = useCallback(async () => {
    if (!meetingId) return;
    setIsLoading(true);
    try {
      const list = await invokeTauri<WorkflowRun[]>('api_list_workflow_runs', { meetingId });
      setRuns(list);
    } catch (err) {
      console.error('Failed to list workflow runs:', err);
    } finally {
      setIsLoading(false);
    }
  }, [meetingId]);

  useEffect(() => {
    refresh();
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [refresh]);

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
    setActiveRunId(null);
  }, []);

  const startPolling = useCallback((runId: string) => {
    if (pollRef.current) clearInterval(pollRef.current);
    setActiveRunId(runId);
    let count = 0;
    pollRef.current = setInterval(async () => {
      count += 1;
      if (count >= MAX_POLLS) {
        stopPolling();
        toast.error('Workflow run timed out');
        await refresh();
        return;
      }
      try {
        const run = await invokeTauri<WorkflowRun | null>('api_get_workflow_run', { runId });
        await refresh();
        if (run && TERMINAL.includes(run.status)) {
          stopPolling();
          if (run.status === 'completed') toast.success('Workflow run completed');
          else if (run.status === 'error') toast.error(`Workflow run failed: ${run.error ?? 'unknown error'}`);
        }
      } catch (err) {
        console.error('Polling workflow run failed:', err);
        stopPolling();
      }
    }, POLL_MS);
  }, [refresh, stopPolling]);

  const runWorkflow = useCallback(async (workflowId: string, text: string, summaryLanguage?: string | null) => {
    if (!meetingId) return;
    if (!text.trim()) {
      toast.error('No transcript text available for this meeting');
      return;
    }
    try {
      const { runId } = await invokeTauri<{ runId: string }>('api_run_workflow', {
        workflowId,
        meetingId,
        text,
        summaryLanguage: summaryLanguage ?? null,
      });
      toast.info('Workflow started');
      await refresh();
      startPolling(runId);
    } catch (err) {
      console.error('Failed to run workflow:', err);
      toast.error(`Failed to run workflow: ${err instanceof Error ? err.message : String(err)}`);
    }
  }, [meetingId, refresh, startPolling]);

  const cancelRun = useCallback(async (runId: string) => {
    try {
      await invokeTauri<boolean>('api_cancel_workflow_run', { runId });
      stopPolling();
      await refresh();
    } catch (err) {
      console.error('Failed to cancel run:', err);
    }
  }, [refresh, stopPolling]);

  const exportRun = useCallback(async (runId: string): Promise<ExportResult | null> => {
    try {
      const result = await invokeTauri<ExportResult>('api_export_run_to_neohive', { runId });
      if (result.failed === 0) toast.success(`Sent ${result.pushed} sections to NeoHive`);
      else toast.warning(`Sent ${result.pushed}, ${result.failed} failed`);
      await refresh();
      return result;
    } catch (err) {
      console.error('Failed to export run:', err);
      toast.error(`NeoHive export failed: ${err instanceof Error ? err.message : String(err)}`);
      return null;
    }
  }, [refresh]);

  return { runs, isLoading, refresh, runWorkflow, cancelRun, exportRun, activeRunId };
}
```

- [ ] **Step 2: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/hooks/meeting-details/useWorkflowRuns.ts
git commit -m "feat(workflows): :sparkles: add useWorkflowRuns hook with run polling"
```

---

## Phase 3 — Settings: Workflows manager

### Task 5: `ModelPicker` (provider + model sub-picker)

**Files:**
- Create: `frontend/src/components/workflows/ModelPicker.tsx`

**Interfaces:**
- Consumes: `useConfig()` (`modelOptions`, `models`), `get_openrouter_models` invoke (reused from `ModelSettingsModal`).
- Produces: `<ModelPicker provider model onProviderChange onModelChange />`.

**Rationale:** the workflow editor needs the same provider+model selection as summaries. Rather than couple to global config, this compact picker takes controlled `provider`/`model` and reports changes. OpenRouter models are fetched on demand exactly as `ModelSettingsModal.loadOpenRouterModels` does (`invoke('get_openrouter_models')`).

- [ ] **Step 1: Write the component**

```tsx
'use client';

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from '@/components/ui/command';
import { Check, ChevronsUpDown, RefreshCw } from 'lucide-react';
import { cn } from '@/lib/utils';

const PROVIDERS: Array<{ value: string; label: string }> = [
  { value: 'openrouter', label: 'OpenRouter' },
  { value: 'ollama', label: 'Ollama' },
  { value: 'claude', label: 'Claude' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'groq', label: 'Groq' },
];

const FALLBACKS: Record<string, string[]> = {
  claude: ['claude-sonnet-4-5-20250929', 'claude-haiku-4-5-20251001', 'claude-opus-4-5-20251101'],
  openai: ['gpt-4o', 'gpt-4o-mini', 'o3-mini'],
  groq: ['llama-3.3-70b-versatile', 'mixtral-8x7b-32768'],
};

interface OpenRouterModel { id: string; name: string; }

interface ModelPickerProps {
  provider: string;
  model: string;
  onProviderChange: (provider: string) => void;
  onModelChange: (model: string) => void;
}

export function ModelPicker({ provider, model, onProviderChange, onModelChange }: ModelPickerProps) {
  const [openRouterModels, setOpenRouterModels] = useState<OpenRouterModel[]>([]);
  const [loadingOR, setLoadingOR] = useState(false);
  const [comboOpen, setComboOpen] = useState(false);

  useEffect(() => {
    if (provider === 'openrouter' && openRouterModels.length === 0) {
      setLoadingOR(true);
      invoke<OpenRouterModel[]>('get_openrouter_models')
        .then(setOpenRouterModels)
        .catch((e) => console.error('OpenRouter models load failed:', e))
        .finally(() => setLoadingOR(false));
    }
  }, [provider, openRouterModels.length]);

  const modelList = useMemo<string[]>(() => {
    if (provider === 'openrouter') return openRouterModels.map((m) => m.id);
    return FALLBACKS[provider] ?? [];
  }, [provider, openRouterModels]);

  return (
    <div className="flex gap-2 items-center">
      <Select value={provider} onValueChange={(v) => { onProviderChange(v); onModelChange(''); }}>
        <SelectTrigger className="w-[160px]"><SelectValue placeholder="Provider" /></SelectTrigger>
        <SelectContent>
          {PROVIDERS.map((p) => <SelectItem key={p.value} value={p.value}>{p.label}</SelectItem>)}
        </SelectContent>
      </Select>

      <Popover open={comboOpen} onOpenChange={setComboOpen} modal>
        <PopoverTrigger asChild>
          <Button variant="outline" role="combobox" className="flex-1 max-w-[260px] justify-between font-normal">
            <span className="truncate">{model || 'Select or type model…'}</span>
            <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-[300px] p-0" align="start">
          <Command>
            <CommandInput
              placeholder="Search or type a model id…"
              onValueChange={(v) => onModelChange(v)}
            />
            <CommandList className="max-h-[300px]">
              {loadingOR ? (
                <div className="py-6 text-center text-sm text-muted-foreground">
                  <RefreshCw className="mx-auto h-4 w-4 animate-spin mb-2" /> Loading models…
                </div>
              ) : (
                <>
                  <CommandEmpty>Type a model id and press Enter.</CommandEmpty>
                  <CommandGroup>
                    {modelList.map((m) => (
                      <CommandItem key={m} value={m} onSelect={(v) => { onModelChange(v); setComboOpen(false); }}>
                        <Check className={cn('mr-2 h-4 w-4', model === m ? 'opacity-100' : 'opacity-0')} />
                        <span className="truncate">{m}</span>
                      </CommandItem>
                    ))}
                  </CommandGroup>
                </>
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  );
}
```

- [ ] **Step 2: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean. (If a UI primitive import path differs, align it with `ModelSettingsModal.tsx` imports lines 10–29.)

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/workflows/ModelPicker.tsx
git commit -m "feat(workflows): :sparkles: add provider/model picker for workflow editor"
```

---

### Task 6: `WorkflowEditor` form

**Files:**
- Create: `frontend/src/components/workflows/WorkflowEditor.tsx`

**Interfaces:**
- Consumes: `useTemplates`, `ModelPicker` (Task 5), types from `@/types/workflow`, `DEFAULT_NEOHIVE_EXPORT`.
- Produces: `<WorkflowEditor initial? onSave onCancel />` where `onSave(input: WorkflowInput)`.

**Applies approved defaults (spec §7):** new workflows seed `sectionTypeOverrides` with `{ "Key Decisions": "decision", "Action Items": "decision"?}`... use `{ "Key Decisions": "decision", "Action Items": "insight" }`, `defaultType: "narrative"`, `importance: 6`, export disabled by default.

- [ ] **Step 1: Write the component**

```tsx
'use client';

import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { ModelPicker } from './ModelPicker';
import { useTemplates } from '@/hooks/meeting-details/useTemplates';
import { NeoHiveExportConfig, Workflow, WorkflowInput } from '@/types/workflow';

const DEFAULT_EXPORT: NeoHiveExportConfig = {
  enabled: false,
  autoExport: false,
  sectionTypeOverrides: { 'Key Decisions': 'decision', 'Action Items': 'insight' },
  defaultType: 'narrative',
  importance: 6,
};

interface WorkflowEditorProps {
  initial?: Workflow;
  onSave: (input: WorkflowInput) => Promise<void> | void;
  onCancel: () => void;
}

export function WorkflowEditor({ initial, onSave, onCancel }: WorkflowEditorProps) {
  const { availableTemplates } = useTemplates();
  const [name, setName] = useState(initial?.name ?? '');
  const [description, setDescription] = useState(initial?.description ?? '');
  const [templateId, setTemplateId] = useState(initial?.templateId ?? 'standard_meeting');
  const [customPrompt, setCustomPrompt] = useState(initial?.customPrompt ?? '');
  const [provider, setProvider] = useState(initial?.provider ?? 'openrouter');
  const [model, setModel] = useState(initial?.model ?? '');
  const [exportCfg, setExportCfg] = useState<NeoHiveExportConfig>(() => {
    if (initial?.neohiveExport) {
      try { return JSON.parse(initial.neohiveExport) as NeoHiveExportConfig; } catch { /* fall through */ }
    }
    return DEFAULT_EXPORT;
  });

  const canSave = name.trim() && provider.trim() && model.trim() && templateId.trim();

  const handleSave = async () => {
    if (!canSave) return;
    const input: WorkflowInput = {
      id: initial?.id,
      name: name.trim(),
      description: description.trim() || null,
      templateId,
      customPrompt: customPrompt.trim() || null,
      provider,
      model: model.trim(),
      maxTokens: initial?.maxTokens ?? null,
      temperature: initial?.temperature ?? null,
      topP: initial?.topP ?? null,
      neohiveExport: exportCfg,
    };
    await onSave(input);
  };

  return (
    <div className="space-y-4 border rounded-lg p-4">
      <div className="space-y-1">
        <Label>Name</Label>
        <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Executive summary" />
      </div>

      <div className="space-y-1">
        <Label>Description</Label>
        <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Optional" />
      </div>

      <div className="space-y-1">
        <Label>Template</Label>
        <Select value={templateId} onValueChange={setTemplateId}>
          <SelectTrigger><SelectValue placeholder="Template" /></SelectTrigger>
          <SelectContent>
            {availableTemplates.map((t) => (
              <SelectItem key={t.id} value={t.id}>{t.name}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-1">
        <Label>Model</Label>
        <ModelPicker provider={provider} model={model} onProviderChange={setProvider} onModelChange={setModel} />
      </div>

      <div className="space-y-1">
        <Label>Custom prompt (optional)</Label>
        <textarea
          className="w-full min-h-[80px] rounded-md border p-2 text-sm"
          value={customPrompt}
          onChange={(e) => setCustomPrompt(e.target.value)}
          placeholder="Extra guidance appended to the template instructions"
        />
      </div>

      <div className="flex items-center justify-between border-t pt-3">
        <div>
          <Label>Send results to NeoHive</Label>
          <p className="text-xs text-muted-foreground">Each section becomes its own memory. Manual by default.</p>
        </div>
        <Switch
          checked={exportCfg.enabled}
          onCheckedChange={(v) => setExportCfg((c) => ({ ...c, enabled: v }))}
        />
      </div>

      {exportCfg.enabled && (
        <div className="flex items-center justify-between pl-2">
          <Label className="text-sm">Auto-export when a run completes</Label>
          <Switch
            checked={exportCfg.autoExport}
            onCheckedChange={(v) => setExportCfg((c) => ({ ...c, autoExport: v }))}
          />
        </div>
      )}

      <div className="flex justify-end gap-2 pt-2">
        <Button variant="outline" onClick={onCancel}>Cancel</Button>
        <Button disabled={!canSave} onClick={handleSave}>{initial ? 'Save changes' : 'Create workflow'}</Button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/workflows/WorkflowEditor.tsx
git commit -m "feat(workflows): :sparkles: add workflow create/edit form"
```

---

### Task 7: `WorkflowsSettings` (manager + NeoHive connection) + Settings tab

**Files:**
- Create: `frontend/src/components/workflows/WorkflowsSettings.tsx`
- Modify: `frontend/src/app/settings/page.tsx`

**Interfaces:**
- Consumes: `useWorkflows` (Task 3), `WorkflowEditor` (Task 6), `api_get_neohive_config`/`api_save_neohive_config` (Task 1).
- Produces: the Workflows settings surface + a new tab.

- [ ] **Step 1: Write `WorkflowsSettings.tsx`**

```tsx
'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Plus, Pencil, Trash2, Eye, EyeOff } from 'lucide-react';
import { useWorkflows } from '@/hooks/meeting-details/useWorkflows';
import { WorkflowEditor } from './WorkflowEditor';
import { NeoHiveSettings, Workflow } from '@/types/workflow';

const DEFAULT_ENDPOINT = 'https://neohive.logilica.com/projects/e95faa80-9092-478d-98b0-19ef8158efb8/mcp';

export function WorkflowsSettings() {
  const { workflows, isLoading, saveWorkflow, deleteWorkflow } = useWorkflows();
  const [editing, setEditing] = useState<Workflow | 'new' | null>(null);

  // NeoHive connection
  const [neo, setNeo] = useState<NeoHiveSettings>({ endpoint: null, apiKey: null, enabled: false });
  const [showToken, setShowToken] = useState(false);

  useEffect(() => {
    invoke<NeoHiveSettings>('api_get_neohive_config')
      .then((cfg) => setNeo({ endpoint: cfg.endpoint ?? DEFAULT_ENDPOINT, apiKey: cfg.apiKey ?? '', enabled: cfg.enabled }))
      .catch((e) => console.error('Failed to load NeoHive config:', e));
  }, []);

  const saveNeo = async () => {
    try {
      await invoke('api_save_neohive_config', {
        endpoint: neo.endpoint || null,
        apiKey: neo.apiKey || null,
        enabled: neo.enabled,
      });
      toast.success('NeoHive settings saved');
    } catch (e) {
      toast.error('Failed to save NeoHive settings');
    }
  };

  return (
    <div className="space-y-8 mt-6">
      {/* NeoHive connection */}
      <section className="space-y-3 border rounded-lg p-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="font-medium">NeoHive export</h3>
            <p className="text-xs text-muted-foreground">Where workflow results are sent. Your own infrastructure.</p>
          </div>
          <Switch checked={neo.enabled} onCheckedChange={(v) => setNeo((n) => ({ ...n, enabled: v }))} />
        </div>
        <div className="space-y-1">
          <Label>Endpoint</Label>
          <Input value={neo.endpoint ?? ''} onChange={(e) => setNeo((n) => ({ ...n, endpoint: e.target.value }))} />
        </div>
        <div className="space-y-1">
          <Label>Token</Label>
          <div className="flex gap-2">
            <Input
              type={showToken ? 'text' : 'password'}
              value={neo.apiKey ?? ''}
              onChange={(e) => setNeo((n) => ({ ...n, apiKey: e.target.value }))}
              placeholder="NeoHive access token"
            />
            <Button variant="outline" size="icon" onClick={() => setShowToken((s) => !s)}>
              {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
            </Button>
          </div>
        </div>
        <div className="flex justify-end"><Button onClick={saveNeo}>Save NeoHive settings</Button></div>
      </section>

      {/* Workflows list */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="font-medium">Workflows</h3>
          {editing === null && (
            <Button size="sm" onClick={() => setEditing('new')}><Plus className="h-4 w-4 mr-1" /> New workflow</Button>
          )}
        </div>

        {editing === 'new' && (
          <WorkflowEditor onSave={async (input) => { await saveWorkflow(input); setEditing(null); }} onCancel={() => setEditing(null)} />
        )}

        {isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}

        {workflows.map((wf) => (
          editing !== null && editing !== 'new' && editing.id === wf.id ? (
            <WorkflowEditor key={wf.id} initial={wf} onSave={async (input) => { await saveWorkflow(input); setEditing(null); }} onCancel={() => setEditing(null)} />
          ) : (
            <div key={wf.id} className="flex items-center justify-between border rounded-lg p-3">
              <div>
                <div className="font-medium">{wf.name}</div>
                <div className="text-xs text-muted-foreground">{wf.provider} / {wf.model} · {wf.templateId}</div>
              </div>
              <div className="flex gap-1">
                <Button variant="ghost" size="icon" onClick={() => setEditing(wf)}><Pencil className="h-4 w-4" /></Button>
                <Button variant="ghost" size="icon" onClick={() => deleteWorkflow(wf.id)}><Trash2 className="h-4 w-4" /></Button>
              </div>
            </div>
          )
        ))}

        {!isLoading && workflows.length === 0 && editing === null && (
          <p className="text-sm text-muted-foreground">No workflows yet. Create one to run summaries in different ways.</p>
        )}
      </section>
    </div>
  );
}
```

- [ ] **Step 2: Add the tab in `app/settings/page.tsx`**

Add to the `TABS` array (import a suitable icon, e.g. `Workflow` or `ListChecks` from `lucide-react`):

```tsx
  { value: 'workflows', label: 'Workflows', icon: ListChecks },
```

Add the tab body next to the other `<TabsContent>` blocks:

```tsx
<TabsContent value="workflows"><WorkflowsSettings /></TabsContent>
```

And import: `import { WorkflowsSettings } from '@/components/workflows/WorkflowsSettings';`

- [ ] **Step 3: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean.

- [ ] **Step 4: Manual verify**

Run `./clean_run.sh` from `frontend/`, open Settings → Workflows. Confirm: NeoHive card loads (endpoint prefilled), a workflow can be created (name + template + OpenRouter model), it appears in the list, edit + delete work.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/workflows/WorkflowsSettings.tsx frontend/src/app/settings/page.tsx
git commit -m "feat(workflows): :sparkles: add Workflows settings tab with NeoHive config"
```

---

## Phase 4 — Meeting view: run + results

### Task 8: `WorkflowRunCard`

**Files:**
- Create: `frontend/src/components/MeetingDetails/WorkflowRunCard.tsx`

**Interfaces:**
- Consumes: `WorkflowRun` type, `exportRun` callback.
- Produces: `<WorkflowRunCard run onExport onCancel />` rendering status, markdown, copy + Send-to-NeoHive.

- [ ] **Step 1: Write the component**

```tsx
'use client';

import { useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Copy, Send, Loader2, CheckCircle2, XCircle } from 'lucide-react';
import { WorkflowRun } from '@/types/workflow';

interface WorkflowRunCardProps {
  run: WorkflowRun;
  onExport: (runId: string) => Promise<unknown>;
  onCancel: (runId: string) => Promise<unknown>;
}

export function WorkflowRunCard({ run, onExport, onCancel }: WorkflowRunCardProps) {
  const [exporting, setExporting] = useState(false);
  const inProgress = run.status === 'queued' || run.status === 'running';

  const copy = async () => {
    await navigator.clipboard.writeText(run.resultMarkdown ?? '');
    toast.success('Copied to clipboard');
  };

  const doExport = async () => {
    setExporting(true);
    try { await onExport(run.id); } finally { setExporting(false); }
  };

  return (
    <div className="bg-white border rounded-lg shadow-sm p-4 space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="font-medium">{run.workflowName}</span>
          {run.status === 'completed' && <CheckCircle2 className="h-4 w-4 text-green-600" />}
          {run.status === 'error' && <XCircle className="h-4 w-4 text-red-600" />}
          {inProgress && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />}
          <span className="text-xs text-muted-foreground">{run.status}</span>
          {run.neohiveStatus !== 'none' && (
            <span className="text-xs rounded px-1.5 py-0.5 bg-muted">NeoHive: {run.neohiveStatus}</span>
          )}
        </div>
        <div className="flex gap-1">
          {inProgress && <Button variant="ghost" size="sm" onClick={() => onCancel(run.id)}>Cancel</Button>}
          {run.status === 'completed' && (
            <>
              <Button variant="ghost" size="icon" onClick={copy}><Copy className="h-4 w-4" /></Button>
              <Button variant="outline" size="sm" onClick={doExport} disabled={exporting}>
                {exporting ? <Loader2 className="h-4 w-4 animate-spin mr-1" /> : <Send className="h-4 w-4 mr-1" />}
                Send to NeoHive
              </Button>
            </>
          )}
        </div>
      </div>

      {run.status === 'error' && run.error && <p className="text-sm text-red-600">{run.error}</p>}
      {run.status === 'completed' && (
        <pre className="text-sm whitespace-pre-wrap font-sans max-h-[400px] overflow-y-auto">{run.resultMarkdown}</pre>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/MeetingDetails/WorkflowRunCard.tsx
git commit -m "feat(workflows): :sparkles: add workflow run result card"
```

---

### Task 9: `WorkflowRunSection` (run control + stacked cards)

**Files:**
- Create: `frontend/src/components/MeetingDetails/WorkflowRunSection.tsx`

**Interfaces:**
- Consumes: `useWorkflows`, `useWorkflowRuns` (Tasks 3/4), `WorkflowRunCard` (Task 8).
- Produces: `<WorkflowRunSection meetingId transcriptText summaryLanguage? />` — self-contained.

- [ ] **Step 1: Write the component**

```tsx
'use client';

import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { ChevronDown, Play } from 'lucide-react';
import { useWorkflows } from '@/hooks/meeting-details/useWorkflows';
import { useWorkflowRuns } from '@/hooks/meeting-details/useWorkflowRuns';
import { WorkflowRunCard } from './WorkflowRunCard';

interface WorkflowRunSectionProps {
  meetingId: string;
  transcriptText: string;
  summaryLanguage?: string | null;
}

export function WorkflowRunSection({ meetingId, transcriptText, summaryLanguage }: WorkflowRunSectionProps) {
  const { workflows } = useWorkflows();
  const { runs, runWorkflow, cancelRun, exportRun } = useWorkflowRuns(meetingId);

  return (
    <div className="p-6 w-full space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium text-muted-foreground">Workflows</h3>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="sm" variant="outline" disabled={workflows.length === 0}>
              <Play className="h-4 w-4 mr-1" /> Run workflow <ChevronDown className="h-4 w-4 ml-1" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {workflows.map((wf) => (
              <DropdownMenuItem key={wf.id} onClick={() => runWorkflow(wf.id, transcriptText, summaryLanguage)}>
                {wf.name}
              </DropdownMenuItem>
            ))}
            {workflows.length === 0 && <DropdownMenuItem disabled>No workflows — create one in Settings</DropdownMenuItem>}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <div className="space-y-3">
        {runs.map((run) => (
          <WorkflowRunCard key={run.id} run={run} onExport={exportRun} onCancel={cancelRun} />
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean. (Confirm a `dropdown-menu` primitive exists under `@/components/ui/`; if not, use the `Popover` + a simple list as in `ModelPicker`.)

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/MeetingDetails/WorkflowRunSection.tsx
git commit -m "feat(workflows): :sparkles: add run control + stacked run cards"
```

---

### Task 10: Mount `WorkflowRunSection` in `SummaryPanel`

**Files:**
- Modify: `frontend/src/components/MeetingDetails/SummaryPanel.tsx`

**Interfaces:**
- Consumes: `WorkflowRunSection` (Task 9). `SummaryPanel` already receives `meeting` and `transcripts` props.

- [ ] **Step 1: Import + derive transcript text + render the section**

Add the import at the top:

```tsx
import { WorkflowRunSection } from './WorkflowRunSection';
```

Inside the scroll container, immediately after the `<div className="p-6 w-full"> …BlockNoteSummaryView… </div>` block (ground-truth: around line 433), insert:

```tsx
          <WorkflowRunSection
            meetingId={meeting.id}
            transcriptText={transcripts.map((t) => t.text).join('\n')}
            summaryLanguage={null}
          />
```

(If a `summaryLang` value is already in scope in `SummaryPanel` — it is, per ground-truth `SummaryPanel` state `summaryLang` — pass that instead of `null`.)

- [ ] **Step 2: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: clean.

- [ ] **Step 3: Manual verify**

Run `./clean_run.sh`, open a meeting with transcripts. Confirm the "Run workflow ▾" control appears, running a workflow shows a card that transitions queued → running → completed (polling), the markdown renders, and "Send to NeoHive" works.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/MeetingDetails/SummaryPanel.tsx
git commit -m "feat(workflows): :sparkles: surface workflow runs on the meeting view"
```

---

## Phase 5 — End-to-end verification

### Task 11: Full manual E2E walkthrough

**Files:** none (verification).

- [ ] **Step 1: Configure**

Run `./clean_run.sh debug` from `frontend/`. In Settings → Workflows: set the NeoHive endpoint + token, toggle enabled, save. Create a workflow "Exec summary" using OpenRouter + a model, template `standard_meeting`, NeoHive export enabled (manual).

- [ ] **Step 2: Run**

Open a meeting that has transcripts. Click **Run workflow → Exec summary**. Confirm the card appears and transitions to `completed` within the poll window; the markdown renders.

- [ ] **Step 3: Export**

Click **Send to NeoHive** on the completed card. Confirm the success toast (`Sent N sections`), and that the card's NeoHive badge shows `pushed`.

- [ ] **Step 4: Confirm in NeoHive**

In a Claude Code session for this repo, run a `memory_recall` for one of the section tags (e.g. the meeting title or "Action Items") and confirm the exported memory is present. Verify (DevTools console + terminal) that the NeoHive token and any provider API key are never printed.

- [ ] **Step 5: Regression check**

Confirm the existing single-summary generation still works unchanged (generate/regenerate a normal summary), and that deleting a meeting removes its workflow runs (Plan 1 Task 8).

- [ ] **Step 6: Commit (docs, if any notes were captured)**

```bash
git add -A && git commit -m "test(workflows): :white_check_mark: manual E2E verification notes" || true
```

---

## Self-Review

**1. Spec coverage (frontend portions):**
- §8 Workflows manager (list/create/edit/delete, template picker, provider+model picker incl. OpenRouter, NeoHive export panel) → Tasks 3, 5, 6, 7. ✅
- §8 run control on meeting view + stacked labeled run cards with copy/export → Tasks 8, 9, 10. ✅
- §8 hooks `useWorkflows` / `useWorkflowRuns` mirroring existing patterns → Tasks 3, 4. ✅
- §6 NeoHive connection config surface (endpoint/token/enabled, overridable, defaults to Meetily project) → Task 1 (commands) + Task 7 (UI, default endpoint prefilled). ✅
- §7 approved defaults applied at creation (Decisions→decision, Action Items→insight, prose→narrative, manual export w/ optional auto) → Task 6 `DEFAULT_EXPORT`. ✅
- §6 never-silent export (manual button + explicit toasts) → Tasks 4, 8. ✅

**2. Placeholder scan:** No TBD/TODO. Two "confirm the primitive exists" notes (Task 5 UI imports, Task 9 `dropdown-menu`) are verification prompts with a stated fallback (`Popover` list), not placeholders.

**3. Type consistency:** `Workflow`/`WorkflowRun`/`WorkflowInput`/`NeoHiveExportConfig`/`NeoHiveSettings`/`ExportResult` names match across Tasks 2–10 and the Plan 1 backend serde contracts (camelCase). Command names + arg keys match Plan 1's registered commands (`workflowId`, `meetingId`, `runId`, `text`, `summaryLanguage`, `workflow`). Hook return shapes match component consumption.

**Deviations / dependencies called out:**
- **Depends on Plan 1** for every `api_*` workflow command; Task 1 adds the only backend piece Plan 1 didn't (NeoHive config commands).
- **No unit tests** for frontend by design (no runner in the repo; not in scope). Verification is typecheck + lint + manual E2E (Task 11). All logic that *can* be unit-tested lives in Rust (Plan 1).
- **Run markdown rendering** uses a `<pre>` block for simplicity; a later polish task could reuse `BlockNoteSummaryView` for rich rendering (YAGNI for v1).
