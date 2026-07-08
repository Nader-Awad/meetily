'use client';

import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';
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

  const canSave = Boolean(name.trim() && provider.trim() && model.trim() && templateId.trim());

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
        <Textarea
          className="min-h-[80px]"
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
