# In-App Custom Templates — Design

- **Date:** 2026-07-17
- **Status:** Proposed — autonomous execution per the user's set-and-forget workflow (brainstorm Q&A was the approval gate). Ships directly to `main` + a release, no PR.
- **Author:** Nader Awad (with Claude)
- **Scope:** Rust/Tauri core (`summary/templates` + `summary/template_commands`) + a Next.js manager UI in Settings → Workflows. No DB changes (templates stay file-based).
- **Builds on / relates to:** This is **Feature A** of the agreed sequence (B custom-vocab → Standup workflow → **A in-app template editor**). It makes iterating on templates (including the planned standup template) comfortable without hand-editing JSON on disk.

## 1. Problem & motivation

Summary templates are JSON files. Built-ins ship in the app; custom templates must be hand-placed in `~/Library/Application Support/Meetily/templates/`. There is no in-app way to create or edit one. The user wants to create/edit templates in the app, starting from the built-ins.

## 2. What already exists (baseline — reuse, do NOT rebuild)

Verified on the current tree (v0.6.4):
- **Model** (`summary/templates/types.rs`): `Template { name, description, sections: Vec<TemplateSection> }`; `TemplateSection { title, instruction, format, item_format?, example_item_format? }` — serde field names verbatim (no renames), no `id` on the struct (id = filename). `Template::validate()` enforces: non-empty name/description/sections; per-section non-empty title/instruction and `format ∈ {"paragraph","list","string"}`.
- **Loader** (`summary/templates/loader.rs`): `get_template(id)` resolves **custom → bundled → built-in** (transparent — no source recorded); `validate_and_parse_template(json)`; `list_template_ids()`; `list_templates() -> Vec<(id,name,description)>`. `get_custom_templates_dir()` is **private and only builds the path — never creates the dir**. **No source info anywhere; no save/delete.**
- **Commands** (`summary/template_commands.rs`, registered `lib.rs:697-700`): `api_list_templates -> Vec<TemplateInfo{id,name,description}>`; `api_get_template_details -> {id,name,description, sections: Vec<String>}` (**titles only**); `api_validate_template(json) -> Result<String>`. **No get-full / save / delete.**
- **Frontend**: `useTemplates` (invokes `api_list_templates`, returns `availableTemplates: {id,name,description}[]`, **no refresh**); `WorkflowEditor` maps `availableTemplates` into the template `<Select>`; `WorkflowsSettings.tsx` has a Workflows-list section (add/edit/delete cards via `useState<Workflow|'new'|null>`) that is the pattern to clone; rendered in Settings → `workflows` tab. UI primitives from `@/components/ui/*`; `toast` from `sonner`; icons from `lucide-react`. No array-of-objects (section rows) editor exists yet — built fresh here. No drag-drop lib present.

## 3. Decisions (from brainstorm)

- **Structured form** editor (not raw JSON) — no new deps, guides valid input, matches `WorkflowEditor`.
- **Duplicate & edit built-ins** — saving a custom template with the same id transparently overrides the built-in via the existing fallback; "Duplicate" seeds the form from any template.
- **Lives in Settings → Workflows** — a new "Templates" section inside `WorkflowsSettings.tsx`.

## 4. Design

**Backend (new, in `summary/templates/loader.rs` + `summary/template_commands.rs`):**
- `loader::is_custom_template(id) -> bool` — `custom_dir/<id>.json` exists.
- `loader::save_custom_template_in(dir, id, &Template) -> Result<(),String>` (pure/testable: `create_dir_all(dir)`, pretty-serialize, write `<dir>/<id>.json`) + `loader::save_custom_template(id, &Template)` (resolves the real custom dir → calls `_in`).
- `loader::delete_custom_template_in(dir, id) -> Result<(),String>` + `loader::delete_custom_template(id)` (remove `<dir>/<id>.json`; error if absent).
- `loader::is_valid_template_id(id) -> bool` — non-empty, ≤64 chars, `^[a-z0-9_-]+$` (blocks path traversal / slashes / dots).
- Commands: `api_get_template(id) -> Result<Template>` (full struct, via `get_template`); extend `TemplateInfo` with `is_custom: bool` (`#[serde(rename="isCustom")]`) in `api_list_templates`; `api_save_custom_template(template_id, template: Template) -> Result<()>` (guard `is_valid_template_id`, `template.validate()`, then `save_custom_template`); `api_delete_custom_template(template_id) -> Result<()>` (guard id + custom-only). Register all in `lib.rs`.

**Frontend:**
- TS types `Template`/`TemplateSection` (verbatim field names) in `frontend/src/types/template.ts`.
- `useTemplates`: add `refresh()`, and carry `isCustom` on `availableTemplates` items (additive).
- `TemplateEditor.tsx` — structured form: `id` (text; editable for new/duplicate, fixed when editing an existing custom), `name`, `description`, and a **sections** array (add / remove / move up-down; each row: `title`, `instruction`, `format` `<Select>` paragraph|list|string, optional `item_format`). Preserve `example_item_format` on loaded sections (passthrough, not exposed). Save → `api_save_custom_template`; surface the Rust validator's error message inline + `toast`.
- Templates manager section in `WorkflowsSettings.tsx` (clone the Workflows-list pattern): list from `api_list_templates` (now with `isCustom`); **Duplicate** on any (seeds editor via `api_get_template`, suggests id `<orig>_custom`); **Edit + Delete** on custom only; **New template** (blank + one empty section). After save/delete, `refresh()`.

**id handling:** derive a default id from the name (lowercase, non-`[a-z0-9]`→`_`, collapse repeats, trim) for New; `<orig>_custom` for Duplicate; validated server-side. Saving an id that already exists overwrites it (edit); saving a built-in's id creates an override (intended).

## 5. Non-goals (YAGNI)

- DB-backed templates — stay file-based (the fallback chain already assumes files).
- Drag-and-drop reorder — up/down buttons only (no new dependency).
- Live cross-component dropdown sync — the manager refreshes its own list; an open Workflow editor picks up changes on reopen.
- Editing `example_item_format` in the form — preserved on round-trip, not exposed.
- A raw-JSON editing mode (deferred; structured form only for v1).

## 6. Testing

- Rust unit tests (temp dir): `save_custom_template_in` then read-back parses+validates; `delete_custom_template_in` removes it (and errors when absent); `is_valid_template_id` accepts good ids and rejects `../x`, `a/b`, empty, too-long, uppercase; `is_custom_template` reflects file presence. `create_dir_all` path is exercised by saving into a fresh temp subdir.
- Frontend: `tsc --noEmit` clean; manual — create/duplicate/edit/delete a template in Settings → Workflows and confirm it appears in the Workflow editor's dropdown and produces a summary.
