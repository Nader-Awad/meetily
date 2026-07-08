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
