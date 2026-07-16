'use client';

// Settings panel for the custom vocabulary dictionary (Terms + Corrections).
// Terms bias Whisper transcription (initial_prompt); Corrections are a
// deterministic find-and-replace applied after transcription (every engine);
// Descriptions feed the summary model as a glossary.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Switch } from '@/components/ui/switch';
import { Label } from '@/components/ui/label';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Trash2, Plus } from 'lucide-react';
import { VocabularyConfig, VocabularyEntry, EMPTY_VOCABULARY_CONFIG } from '@/types/vocabulary';

function newEntry(): VocabularyEntry {
  return {
    id: (crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`),
    entryType: 'term',
    text: '',
    replacement: null,
    description: null,
    caseSensitive: false,
    enabled: true,
  };
}

export function VocabularySettings() {
  const [config, setConfig] = useState<VocabularyConfig>(EMPTY_VOCABULARY_CONFIG);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    invoke<VocabularyConfig | null>('api_get_vocabulary_config')
      .then((cfg) => { if (cfg) setConfig(cfg); })
      .catch((e) => console.error('Failed to load vocabulary config:', e));
  }, []);

  const update = (id: string, patch: Partial<VocabularyEntry>) =>
    setConfig((c) => ({ ...c, entries: c.entries.map((e) => (e.id === id ? { ...e, ...patch } : e)) }));
  const remove = (id: string) =>
    setConfig((c) => ({ ...c, entries: c.entries.filter((e) => e.id !== id) }));
  const add = () => setConfig((c) => ({ ...c, entries: [...c.entries, newEntry()] }));

  const save = async () => {
    setSaving(true);
    try {
      // Drop blank rows before persisting.
      const entries = config.entries.filter((e) => e.text.trim().length > 0);
      await invoke('api_save_vocabulary_config', { config: { ...config, entries } });
      setConfig((c) => ({ ...c, entries }));
      toast.success('Vocabulary saved');
    } catch (e) {
      console.error('Failed to save vocabulary:', e);
      toast.error('Failed to save vocabulary');
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="mt-6 border-t pt-4 space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <Label className="text-sm font-medium text-gray-700">Custom vocabulary</Label>
          <p className="text-xs text-gray-500 mt-1">
            Terms bias Whisper transcription; Corrections replace mis-heard text (every engine);
            Descriptions are given to the summary model as a glossary.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Label className="text-xs">Enabled</Label>
          <Switch checked={config.enabled} onCheckedChange={(v) => setConfig((c) => ({ ...c, enabled: v }))} />
        </div>
      </div>

      <div className="space-y-2">
        {config.entries.map((e) => (
          <div key={e.id} className="flex flex-wrap items-center gap-2 border rounded-md p-2">
            <Select value={e.entryType} onValueChange={(v) => update(e.id, { entryType: v as VocabularyEntry['entryType'] })}>
              <SelectTrigger className="w-[130px]"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="term">Term</SelectItem>
                <SelectItem value="correction">Correction</SelectItem>
              </SelectContent>
            </Select>
            <Input
              className="w-[160px]"
              placeholder={e.entryType === 'correction' ? 'Mis-heard text' : 'Term'}
              value={e.text}
              onChange={(ev) => update(e.id, { text: ev.target.value })}
            />
            {e.entryType === 'correction' && (
              <Input
                className="w-[160px]"
                placeholder="Correct text"
                value={e.replacement ?? ''}
                onChange={(ev) => update(e.id, { replacement: ev.target.value || null })}
              />
            )}
            <Input
              className="flex-1 min-w-[160px]"
              placeholder="Description (optional — used by the summary glossary)"
              value={e.description ?? ''}
              onChange={(ev) => update(e.id, { description: ev.target.value || null })}
            />
            {e.entryType === 'correction' && (
              <div className="flex items-center gap-1">
                <Label className="text-xs">Aa</Label>
                <Switch checked={e.caseSensitive} onCheckedChange={(v) => update(e.id, { caseSensitive: v })} />
              </div>
            )}
            <Switch checked={e.enabled} onCheckedChange={(v) => update(e.id, { enabled: v })} />
            <Button variant="ghost" size="icon" onClick={() => remove(e.id)}><Trash2 className="h-4 w-4" /></Button>
          </div>
        ))}

        {config.entries.length === 0 && (
          <p className="text-xs text-gray-500">No vocabulary entries yet. Add a term or correction below.</p>
        )}
      </div>

      <div className="flex justify-between">
        <Button variant="outline" size="sm" onClick={add}><Plus className="h-4 w-4 mr-1" /> Add entry</Button>
        <Button size="sm" onClick={save} disabled={saving}>{saving ? 'Saving…' : 'Save vocabulary'}</Button>
      </div>
    </section>
  );
}
