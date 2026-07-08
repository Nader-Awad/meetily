'use client';

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from '@/components/ui/command';
import { Check, ChevronsUpDown, RefreshCw } from 'lucide-react';
import { cn } from '@/lib/utils';

const PROVIDERS: Array<{ value: string; label: string }> = [
  { value: 'openrouter', label: 'OpenRouter' },
  { value: 'ollama', label: 'Ollama' },
  { value: 'claude', label: 'Claude' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'groq', label: 'Groq' },
];

const FALLBACKS: Record<string, string[]> = {
  claude: ['claude-sonnet-4-5-20250929', 'claude-haiku-4-5-20251001', 'claude-opus-4-5-20251101'],
  openai: ['gpt-4o', 'gpt-4o-mini', 'o3-mini'],
  groq: ['llama-3.3-70b-versatile', 'mixtral-8x7b-32768'],
};

interface OpenRouterModel { id: string; name: string; }

interface ModelPickerProps {
  provider: string;
  model: string;
  onProviderChange: (provider: string) => void;
  onModelChange: (model: string) => void;
}

export function ModelPicker({ provider, model, onProviderChange, onModelChange }: ModelPickerProps) {
  const [openRouterModels, setOpenRouterModels] = useState<OpenRouterModel[]>([]);
  const [loadingOR, setLoadingOR] = useState(false);
  const [comboOpen, setComboOpen] = useState(false);

  useEffect(() => {
    if (provider === 'openrouter' && openRouterModels.length === 0) {
      setLoadingOR(true);
      invoke<OpenRouterModel[]>('get_openrouter_models')
        .then(setOpenRouterModels)
        .catch((e) => console.error('OpenRouter models load failed:', e))
        .finally(() => setLoadingOR(false));
    }
  }, [provider, openRouterModels.length]);

  const modelList = useMemo<string[]>(() => {
    if (provider === 'openrouter') return openRouterModels.map((m) => m.id);
    return FALLBACKS[provider] ?? [];
  }, [provider, openRouterModels]);

  return (
    <div className="flex gap-2 items-center">
      <Select value={provider} onValueChange={(v) => { onProviderChange(v); onModelChange(''); }}>
        <SelectTrigger className="w-[160px]"><SelectValue placeholder="Provider" /></SelectTrigger>
        <SelectContent>
          {PROVIDERS.map((p) => <SelectItem key={p.value} value={p.value}>{p.label}</SelectItem>)}
        </SelectContent>
      </Select>

      <Popover open={comboOpen} onOpenChange={setComboOpen} modal>
        <PopoverTrigger asChild>
          <Button variant="outline" role="combobox" className="flex-1 max-w-[260px] justify-between font-normal">
            <span className="truncate">{model || 'Select or type model…'}</span>
            <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-[300px] p-0" align="start">
          <Command>
            <CommandInput
              placeholder="Search or type a model id…"
              onValueChange={(v) => onModelChange(v)}
            />
            <CommandList className="max-h-[300px]">
              {loadingOR ? (
                <div className="py-6 text-center text-sm text-muted-foreground">
                  <RefreshCw className="mx-auto h-4 w-4 animate-spin mb-2" /> Loading models…
                </div>
              ) : (
                <>
                  <CommandEmpty>Type a model id and press Enter.</CommandEmpty>
                  <CommandGroup>
                    {modelList.map((m) => (
                      <CommandItem key={m} value={m} onSelect={(v) => { onModelChange(v); setComboOpen(false); }}>
                        <Check className={cn('mr-2 h-4 w-4', model === m ? 'opacity-100' : 'opacity-0')} />
                        <span className="truncate">{m}</span>
                      </CommandItem>
                    ))}
                  </CommandGroup>
                </>
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  );
}
