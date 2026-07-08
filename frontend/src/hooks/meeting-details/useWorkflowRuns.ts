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
  const activeRunIdRef = useRef<string | null>(null);

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

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
    setActiveRunId(null);
    activeRunIdRef.current = null;
  }, []);

  useEffect(() => {
    refresh();
    return () => {
      stopPolling();
    };
  }, [refresh, stopPolling]);

  const startPolling = useCallback((runId: string) => {
    if (pollRef.current) clearInterval(pollRef.current);
    setActiveRunId(runId);
    activeRunIdRef.current = runId;
    let count = 0;
    pollRef.current = setInterval(async () => {
      if (activeRunIdRef.current !== runId) {
        if (pollRef.current) clearInterval(pollRef.current);
        return;
      }
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
      if (activeRunIdRef.current === runId) stopPolling();
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
