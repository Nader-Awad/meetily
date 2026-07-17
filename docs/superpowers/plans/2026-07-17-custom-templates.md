# In-App Custom Templates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Let users create/edit/duplicate/delete summary templates in-app (structured form) in Settings → Workflows, reusing the existing file-based loader + validator.

**Architecture:** Add file-based CRUD to the existing `summary/templates` loader (the custom-dir → bundled → built-in fallback already makes same-id custom files override built-ins) + Tauri commands, then a structured-form editor + manager section in the frontend. No DB, no new dependencies.

**Tech Stack:** Rust/Tauri (serde, std::fs), Next.js/TypeScript + shadcn UI.

## Global Constraints

- **No new dependencies** (no code-editor lib, no drag-drop lib — reorder via up/down buttons).
- **Templates stay file-based** in the custom dir (`dirs::data_dir()/Meetily/templates/<id>.json`); reuse `Template::validate()` / `validate_and_parse_template` — do not reinvent validation.
- **Template id = filename** → must be sanitized server-side (`^[a-z0-9_-]+$`, ≤64, non-empty) to block path traversal.
- **serde field names are verbatim** (no renames) except the new `TemplateInfo.is_custom` → `#[serde(rename="isCustom")]`. TS mirrors must use `item_format`/`example_item_format` (snake_case).
- **The custom dir is never created today** — the save path must `create_dir_all` first.
- Commit style: gitmoji conventional; **no `Co-Authored-By` / AI attribution**.
- Ships to `main` directly + a release; no PR.

---

## File Structure

**Modify:**
- `frontend/src-tauri/src/summary/templates/loader.rs` — add `is_valid_template_id`, `is_custom_template`, `save_custom_template(_in)`, `delete_custom_template(_in)` + tests.
- `frontend/src-tauri/src/summary/template_commands.rs` — add `api_get_template`, `api_save_custom_template`, `api_delete_custom_template`; add `is_custom` to `TemplateInfo`.
- `frontend/src-tauri/src/lib.rs` — register the 3 new commands.
- `frontend/src/hooks/meeting-details/useTemplates.ts` — add `refresh()`, carry `isCustom`.
- `frontend/src/components/workflows/WorkflowsSettings.tsx` — add a Templates manager section.

**Create:**
- `frontend/src/types/template.ts` — `Template`, `TemplateSection`, `TemplateInfo`.
- `frontend/src/components/workflows/TemplateEditor.tsx` — structured form editor.

---

## Task 1: Backend — loader CRUD + commands (Rust, TDD)

**Files:** Modify `loader.rs`, `template_commands.rs`, `lib.rs`.

**Interfaces produced:**
- `loader::is_valid_template_id(&str) -> bool`, `is_custom_template(&str) -> bool`, `save_custom_template(&str, &Template) -> Result<(),String>`, `delete_custom_template(&str) -> Result<(),String>` (+ `_in(dir,…)` testable variants).
- Commands `api_get_template(template_id) -> Result<Template>`, `api_save_custom_template(template_id, template: Template) -> Result<()>`, `api_delete_custom_template(template_id) -> Result<()>`; `TemplateInfo` gains `is_custom` (`isCustom`).

- [ ] **Step 1: Add loader functions** to `frontend/src-tauri/src/summary/templates/loader.rs` (add `use std::path::Path;` near the top imports; `Template` is already imported):

```rust
/// Valid custom-template id = filename stem. Restricted to prevent path traversal.
pub fn is_valid_template_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// True if a user-authored custom template file exists for this id.
pub fn is_custom_template(id: &str) -> bool {
    get_custom_templates_dir()
        .map(|d| d.join(format!("{id}.json")).exists())
        .unwrap_or(false)
}

/// Testable core: write `<dir>/<id>.json` (creating `dir` if needed).
pub fn save_custom_template_in(dir: &Path, id: &str, template: &Template) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create templates dir: {e}"))?;
    let json = serde_json::to_string_pretty(template).map_err(|e| format!("Failed to serialize template: {e}"))?;
    std::fs::write(dir.join(format!("{id}.json")), json).map_err(|e| format!("Failed to write template '{id}': {e}"))?;
    Ok(())
}

/// Write a custom template into the user's custom templates directory.
pub fn save_custom_template(id: &str, template: &Template) -> Result<(), String> {
    let dir = get_custom_templates_dir().ok_or_else(|| "Could not resolve custom templates directory".to_string())?;
    save_custom_template_in(&dir, id, template)
}

/// Testable core: remove `<dir>/<id>.json`; err if absent.
pub fn delete_custom_template_in(dir: &Path, id: &str) -> Result<(), String> {
    let path = dir.join(format!("{id}.json"));
    if !path.exists() {
        return Err(format!("Custom template '{id}' does not exist"));
    }
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete template '{id}': {e}"))
}

/// Delete a custom template from the user's custom templates directory.
pub fn delete_custom_template(id: &str) -> Result<(), String> {
    let dir = get_custom_templates_dir().ok_or_else(|| "Could not resolve custom templates directory".to_string())?;
    delete_custom_template_in(&dir, id)
}
```

- [ ] **Step 2: Add tests** inside the existing `#[cfg(test)] mod tests` in `loader.rs` (it already has `use super::*;`):

```rust
    #[test]
    fn valid_template_id_rules() {
        assert!(is_valid_template_id("daily_standup"));
        assert!(is_valid_template_id("my-tpl-1"));
        assert!(!is_valid_template_id(""));
        assert!(!is_valid_template_id("../evil"));
        assert!(!is_valid_template_id("a/b"));
        assert!(!is_valid_template_id("Has Space"));
        assert!(!is_valid_template_id("UPPER"));
        assert!(!is_valid_template_id(&"x".repeat(65)));
    }

    #[test]
    fn custom_template_save_read_delete_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("meetily_tpl_test_{}", uuid::Uuid::new_v4()));
        let json = r#"{"name":"T","description":"d","sections":[{"title":"S","instruction":"i","format":"list"}]}"#;
        let tpl = validate_and_parse_template(json).expect("valid");
        save_custom_template_in(&tmp, "my_tpl", &tpl).expect("save ok (creates dir)");
        let content = std::fs::read_to_string(tmp.join("my_tpl.json")).expect("file written");
        let parsed = validate_and_parse_template(&content).expect("round-trips + validates");
        assert_eq!(parsed.name, "T");
        assert_eq!(parsed.sections.len(), 1);
        delete_custom_template_in(&tmp, "my_tpl").expect("delete ok");
        assert!(!tmp.join("my_tpl.json").exists());
        assert!(delete_custom_template_in(&tmp, "my_tpl").is_err()); // gone now
        let _ = std::fs::remove_dir_all(&tmp);
    }
```

(If `uuid` isn't already a dev-usable dep in this crate, use `std::time::SystemTime` nanos for the temp name instead — but `Uuid::new_v4()` is used across `src/` so it is available.)

- [ ] **Step 3: Run the loader tests — verify pass**

Run: `cd frontend/src-tauri && cargo test --lib summary::templates::loader 2>&1 | tail -20`
Expected: new tests PASS (incl. the `create_dir_all` path via a fresh temp dir).

- [ ] **Step 4: Add commands** to `frontend/src-tauri/src/summary/template_commands.rs`. Add an import for the full model — use the path the file already uses to reach templates (confirm: `crate::summary::templates::types::Template`, or `templates::types::Template` if `templates` is imported). Extend `TemplateInfo` and add commands:

```rust
// TemplateInfo: add the source flag (additive; existing consumers ignore it).
//   pub is_custom: bool,  with  #[serde(rename = "isCustom")]
// In api_list_templates, set it per row: is_custom: templates::is_custom_template(&id)

#[tauri::command]
pub async fn api_get_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    template_id: String,
) -> Result<crate::summary::templates::types::Template, String> {
    crate::summary::templates::get_template(&template_id)
}

#[tauri::command]
pub async fn api_save_custom_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    template_id: String,
    template: crate::summary::templates::types::Template,
) -> Result<(), String> {
    if !crate::summary::templates::is_valid_template_id(&template_id) {
        return Err(format!(
            "Invalid template id '{}': use lowercase letters, digits, '_' or '-' (max 64 chars).",
            template_id
        ));
    }
    template.validate()?;
    crate::summary::templates::save_custom_template(&template_id, &template)
}

#[tauri::command]
pub async fn api_delete_custom_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    template_id: String,
) -> Result<(), String> {
    if !crate::summary::templates::is_valid_template_id(&template_id) {
        return Err(format!("Invalid template id '{}'.", template_id));
    }
    crate::summary::templates::delete_custom_template(&template_id)
}
```

(Confirm the actual re-export path for the loader fns — the file already calls `templates::list_templates()` etc., so `templates::is_valid_template_id` / `save_custom_template` / `delete_custom_template` / `get_template` should resolve the same way; and `templates::types::Template` or a `templates::Template` re-export for the struct. Match whatever `api_get_template_details` uses to reach the model. Also confirm `Template::validate` is `pub`.)

- [ ] **Step 5: Register commands** in `frontend/src-tauri/src/lib.rs` next to the existing template commands (~L697-700):

```rust
            summary::template_commands::api_get_template,
            summary::template_commands::api_save_custom_template,
            summary::template_commands::api_delete_custom_template,
```

- [ ] **Step 6: Build**

Run: `cd frontend/src-tauri && cargo build 2>&1 | tail -25`
Expected: clean (no new errors).

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/src/summary/templates/loader.rs frontend/src-tauri/src/summary/template_commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(templates): :sparkles: file-based custom-template CRUD commands + loader helpers"
```

---

## Task 2: Frontend types + `useTemplates` (refresh + isCustom)

**Files:** Create `frontend/src/types/template.ts`; Modify `frontend/src/hooks/meeting-details/useTemplates.ts`.

**Interfaces:** produces `Template`/`TemplateSection`/`TemplateInfo` types; `useTemplates()` now returns `availableTemplates: TemplateInfo[]` (with `isCustom`) plus `refresh: () => Promise<void>`.

- [ ] **Step 1: Create `frontend/src/types/template.ts`**

```ts
// Mirrors crate::summary::templates::types (verbatim field names) + TemplateInfo.
export type TemplateFormat = 'paragraph' | 'list' | 'string';

export interface TemplateSection {
  title: string;
  instruction: string;
  format: TemplateFormat;
  item_format?: string | null;
  example_item_format?: string | null;
}

export interface Template {
  name: string;
  description: string;
  sections: TemplateSection[];
}

export interface TemplateInfo {
  id: string;
  name: string;
  description: string;
  isCustom: boolean;
}
```

- [ ] **Step 2: Extend `useTemplates.ts`** — read the current file first. Move the `invoke('api_list_templates')` call into a `refresh` `useCallback` (mirroring `useWorkflows.ts`'s `refresh`), call it in the mount `useEffect`, type the list as `TemplateInfo[]`, and return `refresh` alongside the existing values. Preserve the existing return keys (`availableTemplates`, `selectedTemplate`, `handleTemplateSelection`) so `WorkflowEditor` keeps working.

```tsx
import { TemplateInfo } from '@/types/template';
// ...
const [availableTemplates, setAvailableTemplates] = useState<TemplateInfo[]>([]);

const refresh = useCallback(async () => {
  try {
    const list = await invoke<TemplateInfo[]>('api_list_templates');
    setAvailableTemplates(list);
  } catch (e) {
    console.error('Failed to load templates:', e);
  }
}, []);

useEffect(() => { refresh(); }, [refresh]);
// ...
return { availableTemplates, selectedTemplate, handleTemplateSelection, refresh };
```

- [ ] **Step 3: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors (a pre-existing `bun:test` error is unrelated).

- [ ] **Step 4: Commit**

```bash
git add frontend/src/types/template.ts frontend/src/hooks/meeting-details/useTemplates.ts
git commit -m "feat(templates): :sparkles: template types + useTemplates refresh/isCustom"
```

---

## Task 3: `TemplateEditor` structured-form component

**Files:** Create `frontend/src/components/workflows/TemplateEditor.tsx`.

**Interfaces:** `TemplateEditor({ initialId, initialTemplate, idFixed, onSaved, onCancel })` — `initialId?: string`, `initialTemplate?: Template`, `idFixed?: boolean` (true when editing an existing custom template), `onSaved: () => void`, `onCancel: () => void`.

- [ ] **Step 1: Write the component** (mirror `WorkflowEditor.tsx` for imports/primitives/layout):

```tsx
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
```

- [ ] **Step 2: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors. (Align UI primitive import paths with `WorkflowEditor.tsx` if any differ.)

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/workflows/TemplateEditor.tsx
git commit -m "feat(templates): :sparkles: structured-form template editor component"
```

---

## Task 4: Templates manager section in `WorkflowsSettings.tsx`

**Files:** Modify `frontend/src/components/workflows/WorkflowsSettings.tsx`.

**Interfaces:** consumes `useTemplates` (list w/ `isCustom` + `refresh`), `api_get_template`, `api_delete_custom_template`, and `TemplateEditor`.

- [ ] **Step 1: Add a Templates section** (clone the existing Workflows-list section pattern, ~L205-239). Read the file first. Add near the top: `import { TemplateEditor } from './TemplateEditor';`, `import { useTemplates } from '@/hooks/meeting-details/useTemplates';`, `import { Template, TemplateInfo } from '@/types/template';`, and `invoke`/`toast` if not already imported. Inside the component:

```tsx
  const { availableTemplates, refresh: refreshTemplates } = useTemplates();
  // editing state: null | 'new' | { id, template, idFixed }
  const [tplEdit, setTplEdit] = useState<null | 'new' | { id: string; template: Template; idFixed: boolean }>(null);

  const startDuplicate = async (info: TemplateInfo) => {
    try {
      const template = await invoke<Template>('api_get_template', { templateId: info.id });
      setTplEdit({ id: `${info.id}_custom`, template, idFixed: false });
    } catch (e) { toast.error('Failed to load template'); console.error(e); }
  };
  const startEdit = async (info: TemplateInfo) => {
    try {
      const template = await invoke<Template>('api_get_template', { templateId: info.id });
      setTplEdit({ id: info.id, template, idFixed: true });
    } catch (e) { toast.error('Failed to load template'); console.error(e); }
  };
  const deleteTemplate = async (info: TemplateInfo) => {
    try { await invoke('api_delete_custom_template', { templateId: info.id }); toast.success('Template deleted'); await refreshTemplates(); }
    catch (e) { toast.error(typeof e === 'string' ? e : 'Failed to delete template'); console.error(e); }
  };
```

Then add a `<section>` after the Workflows section (mirror its markup):

```tsx
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="font-medium">Templates</h3>
          {tplEdit === null && <Button size="sm" onClick={() => setTplEdit('new')}><Plus className="h-4 w-4 mr-1" /> New template</Button>}
        </div>

        {tplEdit === 'new' && (
          <TemplateEditor onSaved={() => { setTplEdit(null); refreshTemplates(); }} onCancel={() => setTplEdit(null)} />
        )}
        {tplEdit !== null && tplEdit !== 'new' && (
          <TemplateEditor
            initialId={tplEdit.id}
            initialTemplate={tplEdit.template}
            idFixed={tplEdit.idFixed}
            onSaved={() => { setTplEdit(null); refreshTemplates(); }}
            onCancel={() => setTplEdit(null)}
          />
        )}

        {tplEdit === null && availableTemplates.map((t) => (
          <div key={t.id} className="flex items-center justify-between border rounded-lg p-3">
            <div className="text-sm">
              <span className="font-medium">{t.name}</span>{' '}
              <span className="text-muted-foreground">({t.id}{t.isCustom ? ', custom' : ', built-in'})</span>
            </div>
            <div className="flex gap-1">
              <Button variant="ghost" size="sm" onClick={() => startDuplicate(t)}>Duplicate</Button>
              {t.isCustom && <Button variant="ghost" size="icon" aria-label="Edit template" onClick={() => startEdit(t)}><Pencil className="h-4 w-4" /></Button>}
              {t.isCustom && <Button variant="ghost" size="icon" aria-label="Delete template" onClick={() => deleteTemplate(t)}><Trash2 className="h-4 w-4" /></Button>}
            </div>
          </div>
        ))}
      </section>
```

(`Plus`, `Pencil`, `Trash2` are already imported in this file. If `invoke`/`toast` aren't imported yet, add them.)

- [ ] **Step 2: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/workflows/WorkflowsSettings.tsx
git commit -m "feat(templates): :sparkles: template manager (list/new/duplicate/edit/delete) in Settings"
```

---

## Self-Review

**Spec coverage:** backend CRUD + source flag + full-get (T1), types + hook refresh/isCustom (T2), structured editor with add/remove/reorder + id derivation + validation surfacing + example_item_format passthrough (T3), manager with duplicate-any / edit+delete-custom + refresh (T4). Non-goals (DB, drag-drop, live cross-component sync, raw-JSON mode, editing example_item_format) excluded.

**Placeholder scan:** backend + types + editor ship complete code; frontend integration tasks give exact code + the mirror file. Line refs are "~" and must be confirmed against the file.

**Type consistency:** `api_save_custom_template` arg keys `templateId`/`template`; `api_get_template`/`api_delete_custom_template` use `templateId`; `TemplateInfo.isCustom`; `Template`/`TemplateSection` verbatim snake_case for `item_format`/`example_item_format` — consistent across T1–T4 and the TS mirror.
