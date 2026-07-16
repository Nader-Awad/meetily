// Mirrors crate::vocabulary::{VocabularyEntry, VocabularyConfig} (serde camelCase).
export type VocabularyEntryType = 'term' | 'correction';

export interface VocabularyEntry {
  id: string;
  entryType: VocabularyEntryType;
  text: string;
  replacement?: string | null;
  description?: string | null;
  caseSensitive: boolean;
  enabled: boolean;
  createdAt?: string | null;
  updatedAt?: string | null;
}

export interface VocabularyConfig {
  enabled: boolean;
  entries: VocabularyEntry[];
}

export const EMPTY_VOCABULARY_CONFIG: VocabularyConfig = { enabled: true, entries: [] };
