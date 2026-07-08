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
