'use client';

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Plus, Trash2, ArrowUp, ArrowDown } from 'lucide-react';
import { Template, TemplateSection, TemplateFormat } from '@/types/template';

function slugify(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 64);
}
function emptySection(): TemplateSection {
  return { title: '', instruction: '', format: 'list', item_format: null, example_item_format: null };
}

interface Props {
  initialId?: string;
  initialTemplate?: Template;
  idFixed?: boolean;
  onSaved: () => void;
  onCancel: () => void;
}

export function TemplateEditor({ initialId, initialTemplate, idFixed, onSaved, onCancel }: Props) {
  const [id, setId] = useState(initialId ?? '');
  const [idTouched, setIdTouched] = useState(Boolean(initialId));
  const [name, setName] = useState(initialTemplate?.name ?? '');
  const [description, setDescription] = useState(initialTemplate?.description ?? '');
  const [sections, setSections] = useState<TemplateSection[]>(
    initialTemplate?.sections?.length ? initialTemplate.sections.map((s) => ({ ...s })) : [emptySection()]
  );
  const [saving, setSaving] = useState(false);

  // Auto-derive id from name for new templates until the user edits id directly.
  const effectiveId = idFixed ? (initialId ?? '') : (idTouched ? id : slugify(name));

  const patchSection = (i: number, p: Partial<TemplateSection>) =>
    setSections((ss) => ss.map((s, idx) => (idx === i ? { ...s, ...p } : s)));
  const addSection = () => setSections((ss) => [...ss, emptySection()]);
  const removeSection = (i: number) => setSections((ss) => ss.filter((_, idx) => idx !== i));
  const move = (i: number, d: -1 | 1) =>
    setSections((ss) => {
      const j = i + d;
      if (j < 0 || j >= ss.length) return ss;
      const copy = [...ss];
      [copy[i], copy[j]] = [copy[j], copy[i]];
      return copy;
    });

  const canSave = Boolean(effectiveId.trim() && name.trim() && description.trim() && sections.length > 0 &&
    sections.every((s) => s.title.trim() && s.instruction.trim()));

  const save = async () => {
    setSaving(true);
    try {
      const template: Template = {
        name: name.trim(),
        description: description.trim(),
        sections: sections.map((s) => ({
          title: s.title.trim(),
          instruction: s.instruction.trim(),
          format: s.format,
          item_format: s.item_format?.trim() ? s.item_format.trim() : null,
          example_item_format: s.example_item_format ?? null, // preserved passthrough
        })),
      };
      await invoke('api_save_custom_template', { templateId: effectiveId, template });
      toast.success(`Template "${name.trim()}" saved`);
      onSaved();
    } catch (e) {
      // Surface the Rust validator's message.
      toast.error(typeof e === 'string' ? e : 'Failed to save template');
      console.error('Save template failed:', e);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-4 border rounded-lg p-4">
      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-1">
          <Label>Name</Label>
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. My Standup" />
        </div>
        <div className="space-y-1">
          <Label>ID {idFixed && <span className="text-xs text-muted-foreground">(fixed)</span>}</Label>
          <Input
            value={effectiveId}
            disabled={idFixed}
            onChange={(e) => { setIdTouched(true); setId(e.target.value); }}
            placeholder="my_standup"
          />
          <p className="text-xs text-muted-foreground">lowercase, digits, _ or - (this is the filename; reusing a built-in id overrides it)</p>
        </div>
      </div>

      <div className="space-y-1">
        <Label>Description</Label>
        <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What this template is for" />
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <Label>Sections</Label>
          <Button variant="outline" size="sm" onClick={addSection}><Plus className="h-4 w-4 mr-1" /> Add section</Button>
        </div>
        {sections.map((s, i) => (
          <div key={i} className="space-y-2 border rounded-md p-2">
            <div className="flex gap-2 items-center">
              <Input className="flex-1" placeholder="Section title" value={s.title} onChange={(e) => patchSection(i, { title: e.target.value })} />
              <Select value={s.format} onValueChange={(v) => patchSection(i, { format: v as TemplateFormat })}>
                <SelectTrigger className="w-[130px]"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="paragraph">paragraph</SelectItem>
                  <SelectItem value="list">list</SelectItem>
                  <SelectItem value="string">string</SelectItem>
                </SelectContent>
              </Select>
              <Button variant="ghost" size="icon" aria-label="Move up" onClick={() => move(i, -1)} disabled={i === 0}><ArrowUp className="h-4 w-4" /></Button>
              <Button variant="ghost" size="icon" aria-label="Move down" onClick={() => move(i, 1)} disabled={i === sections.length - 1}><ArrowDown className="h-4 w-4" /></Button>
              <Button variant="ghost" size="icon" aria-label="Remove section" onClick={() => removeSection(i)}><Trash2 className="h-4 w-4" /></Button>
            </div>
            <Textarea placeholder="Instruction for the LLM (what to extract for this section)" value={s.instruction} onChange={(e) => patchSection(i, { instruction: e.target.value })} />
            <Input placeholder="Optional item format (e.g. | **Owner** | Task | Due |)" value={s.item_format ?? ''} onChange={(e) => patchSection(i, { item_format: e.target.value || null })} />
          </div>
        ))}
      </div>

      <div className="flex justify-end gap-2 pt-2">
        <Button variant="outline" onClick={onCancel}>Cancel</Button>
        <Button disabled={!canSave || saving} onClick={save}>{saving ? 'Saving…' : 'Save template'}</Button>
      </div>
    </div>
  );
}
