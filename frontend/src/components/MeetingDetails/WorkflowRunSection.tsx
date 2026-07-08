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
