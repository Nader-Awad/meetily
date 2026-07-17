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
