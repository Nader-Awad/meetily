'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Plus, Pencil, Trash2, Eye, EyeOff } from 'lucide-react';
import { useWorkflows } from '@/hooks/meeting-details/useWorkflows';
import { WorkflowEditor } from './WorkflowEditor';
import { NeoHiveAuthConfig, NeoHiveSettings, Workflow } from '@/types/workflow';

const DEFAULT_ENDPOINT = 'https://neohive.logilica.com/projects/e95faa80-9092-478d-98b0-19ef8158efb8/mcp';

export function WorkflowsSettings() {
  const { workflows, isLoading, saveWorkflow, deleteWorkflow } = useWorkflows();
  const [editing, setEditing] = useState<Workflow | 'new' | null>(null);

  // NeoHive connection (auth method + method-specific fields)
  const [neo, setNeo] = useState<NeoHiveSettings>({
    endpoint: null, enabled: false, authType: 'cloudflare_access', authConfig: {},
  });
  const [showSecret, setShowSecret] = useState(false);

  const setField = (k: keyof NeoHiveAuthConfig, v: string) =>
    setNeo((n) => ({ ...n, authConfig: { ...n.authConfig, [k]: v } }));

  useEffect(() => {
    invoke<NeoHiveSettings>('api_get_neohive_config')
      .then((cfg) => setNeo({
        endpoint: cfg.endpoint ?? DEFAULT_ENDPOINT,
        enabled: cfg.enabled,
        authType: cfg.authType ?? 'cloudflare_access',
        authConfig: cfg.authConfig ?? {},
      }))
      .catch((e) => console.error('Failed to load NeoHive config:', e));
  }, []);

  const saveNeo = async () => {
    try {
      await invoke('api_save_neohive_config', {
        endpoint: neo.endpoint || null,
        enabled: neo.enabled,
        authType: neo.authType,
        authConfig: neo.authConfig,
      });
      toast.success('NeoHive settings saved');
    } catch (e) {
      console.error('Failed to save NeoHive settings:', e);
      toast.error('Failed to save NeoHive settings');
    }
  };

  return (
    <div className="space-y-8 mt-6">
      {/* NeoHive connection */}
      <section className="space-y-3 border rounded-lg p-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="font-medium">NeoHive export</h3>
            <p className="text-xs text-muted-foreground">
              Connect to your NeoHive project. Your own infrastructure.
            </p>
          </div>
          <Switch checked={neo.enabled} onCheckedChange={(v) => setNeo((n) => ({ ...n, enabled: v }))} />
        </div>

        <div className="space-y-1">
          <Label>Endpoint</Label>
          <Input
            value={neo.endpoint ?? ''}
            onChange={(e) => setNeo((n) => ({ ...n, endpoint: e.target.value }))}
            placeholder={DEFAULT_ENDPOINT}
          />
        </div>

        <div className="space-y-1">
          <Label>Authentication method</Label>
          <select
            className="w-full border rounded-md h-9 px-2 text-sm bg-transparent"
            value={neo.authType}
            onChange={(e) => setNeo((n) => ({ ...n, authType: e.target.value as NeoHiveSettings['authType'] }))}
          >
            <option value="cloudflare_access">Cloudflare Access service token</option>
            <option value="bearer">Bearer token / API key</option>
            <option value="basic">Basic auth (username / password)</option>
            <option value="custom_header">Custom header</option>
            <option value="none">None (network-level, e.g. Tailscale / LAN)</option>
          </select>
        </div>

        {neo.authType === 'cloudflare_access' && (
          <>
            <div className="space-y-1">
              <Label>Access Client Id</Label>
              <Input value={neo.authConfig.clientId ?? ''} onChange={(e) => setField('clientId', e.target.value)} placeholder="xxxxxxxx.access" />
            </div>
            <div className="space-y-1">
              <Label>Access Client Secret</Label>
              <div className="flex gap-2">
                <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.clientSecret ?? ''} onChange={(e) => setField('clientSecret', e.target.value)} placeholder="Cloudflare Access client secret" />
                <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
              </div>
            </div>
          </>
        )}

        {neo.authType === 'bearer' && (
          <div className="space-y-1">
            <Label>Token</Label>
            <div className="flex gap-2">
              <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.token ?? ''} onChange={(e) => setField('token', e.target.value)} placeholder="Bearer token / API key" />
              <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
            </div>
          </div>
        )}

        {neo.authType === 'basic' && (
          <>
            <div className="space-y-1">
              <Label>Username</Label>
              <Input value={neo.authConfig.username ?? ''} onChange={(e) => setField('username', e.target.value)} placeholder="username" />
            </div>
            <div className="space-y-1">
              <Label>Password</Label>
              <div className="flex gap-2">
                <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.password ?? ''} onChange={(e) => setField('password', e.target.value)} placeholder="password" />
                <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
              </div>
            </div>
          </>
        )}

        {neo.authType === 'custom_header' && (
          <>
            <div className="space-y-1">
              <Label>Header name</Label>
              <Input value={neo.authConfig.headerName ?? ''} onChange={(e) => setField('headerName', e.target.value)} placeholder="X-Api-Key" />
            </div>
            <div className="space-y-1">
              <Label>Header value</Label>
              <div className="flex gap-2">
                <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.headerValue ?? ''} onChange={(e) => setField('headerValue', e.target.value)} placeholder="header value" />
                <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
              </div>
            </div>
          </>
        )}

        {neo.authType === 'none' && (
          <p className="text-xs text-muted-foreground">No credentials — Meetily reaches NeoHive over your network (e.g. Tailscale, LAN, or VPN). Only the endpoint is used.</p>
        )}

        <div className="flex justify-end"><Button onClick={saveNeo}>Save NeoHive settings</Button></div>
      </section>

      {/* Workflows list */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="font-medium">Workflows</h3>
          {editing === null && (
            <Button size="sm" onClick={() => setEditing('new')}><Plus className="h-4 w-4 mr-1" /> New workflow</Button>
          )}
        </div>

        {editing === 'new' && (
          <WorkflowEditor onSave={async (input) => { await saveWorkflow(input); setEditing(null); }} onCancel={() => setEditing(null)} />
        )}

        {isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}

        {workflows.map((wf) => (
          editing !== null && editing !== 'new' && editing.id === wf.id ? (
            <WorkflowEditor key={wf.id} initial={wf} onSave={async (input) => { await saveWorkflow(input); setEditing(null); }} onCancel={() => setEditing(null)} />
          ) : (
            <div key={wf.id} className="flex items-center justify-between border rounded-lg p-3">
              <div>
                <div className="font-medium">{wf.name}</div>
                <div className="text-xs text-muted-foreground">{wf.provider} / {wf.model} · {wf.templateId}</div>
              </div>
              <div className="flex gap-1">
                <Button variant="ghost" size="icon" onClick={() => setEditing(wf)}><Pencil className="h-4 w-4" /></Button>
                <Button variant="ghost" size="icon" onClick={() => deleteWorkflow(wf.id)}><Trash2 className="h-4 w-4" /></Button>
              </div>
            </div>
          )
        ))}

        {!isLoading && workflows.length === 0 && editing === null && (
          <p className="text-sm text-muted-foreground">No workflows yet. Create one to run summaries in different ways.</p>
        )}
      </section>
    </div>
  );
}
