import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';
import { Input } from './ui/input';
import { Button } from './ui/button';
import { Label } from './ui/label';
import { Eye, EyeOff, Lock, Unlock, AlertTriangle } from 'lucide-react';
import { ModelManager } from './WhisperModelManager';
import { ParakeetModelManager } from './ParakeetModelManager';


export interface TranscriptModelProps {
    provider: 'localWhisper' | 'parakeet' | 'deepgram' | 'elevenLabs' | 'groq' | 'openai' | 'openrouter' | 'custom';
    model: string;
    apiKey?: string | null;
    baseUrl?: string | null;
}

// Friendly display names for the privacy note shown when a cloud provider is selected.
const CLOUD_PROVIDER_LABELS: Partial<Record<TranscriptModelProps['provider'], string>> = {
    deepgram: 'Deepgram',
    elevenLabs: 'ElevenLabs',
    groq: 'Groq',
    openai: 'OpenAI',
    openrouter: 'OpenRouter',
    custom: 'the custom endpoint',
};

export interface TranscriptSettingsProps {
    transcriptModelConfig: TranscriptModelProps;
    setTranscriptModelConfig: (config: TranscriptModelProps) => void;
    onModelSelect?: () => void;
    onSave?: (config: TranscriptModelProps) => void | Promise<void>;
}

export function TranscriptSettings({ transcriptModelConfig, setTranscriptModelConfig, onModelSelect, onSave }: TranscriptSettingsProps) {
    const [apiKey, setApiKey] = useState<string | null>(transcriptModelConfig.apiKey || null);
    const [baseUrl, setBaseUrl] = useState<string | null>(transcriptModelConfig.baseUrl ?? null);
    const [showApiKey, setShowApiKey] = useState<boolean>(false);
    const [isApiKeyLocked, setIsApiKeyLocked] = useState<boolean>(true);
    const [isLockButtonVibrating, setIsLockButtonVibrating] = useState<boolean>(false);
    const [uiProvider, setUiProvider] = useState<TranscriptModelProps['provider']>(transcriptModelConfig.provider);
    const [isSaving, setIsSaving] = useState<boolean>(false);

    // Sync uiProvider when backend config changes (e.g., after model selection or initial load)
    useEffect(() => {
        setUiProvider(transcriptModelConfig.provider);
    }, [transcriptModelConfig.provider]);

    useEffect(() => {
        if (transcriptModelConfig.provider === 'localWhisper' || transcriptModelConfig.provider === 'parakeet') {
            setApiKey(null);
            setBaseUrl(null);
        }
    }, [transcriptModelConfig.provider]);

    const fetchApiKey = async (provider: string) => {
        try {

            const data = await invoke('api_get_transcript_api_key', { provider }) as string;

            setApiKey(data || '');
        } catch (err) {
            console.error('Error fetching API key:', err);
            setApiKey(null);
        }

        // Base URL is only persisted for the "custom" provider; load it alongside the key.
        if (provider === 'custom') {
            try {
                const config = await invoke('api_get_transcript_config') as TranscriptModelProps | null;
                setBaseUrl(config?.baseUrl ?? null);
            } catch (err) {
                console.error('Error fetching base URL:', err);
            }
        }
    };
    const modelOptions: Record<TranscriptModelProps['provider'], string[]> = {
        localWhisper: [], // Model selection handled by ModelManager component
        parakeet: [], // Model selection handled by ParakeetModelManager component
        deepgram: ['nova-2-phonecall'],
        elevenLabs: ['eleven_multilingual_v2'],
        groq: ['whisper-large-v3-turbo', 'whisper-large-v3'],
        openai: ['whisper-1', 'gpt-4o-transcribe'],
        openrouter: ['openai/whisper-large-v3', 'deepgram/nova-3', 'mistralai/voxtral-mini-transcribe-2602'],
        custom: [], // Free-text model name for OpenAI-compatible custom endpoints
    };
    const requiresApiKey = uiProvider === 'deepgram' || uiProvider === 'elevenLabs' || uiProvider === 'openai' || uiProvider === 'groq' || uiProvider === 'openrouter' || uiProvider === 'custom';
    const isCloudProvider = uiProvider !== 'localWhisper' && uiProvider !== 'parakeet';
    // Model is only "ready to save" once it belongs to the currently selected provider -
    // guards against saving a stale model left over from switching providers before picking a new one.
    const isModelValidForProvider = uiProvider === 'custom'
        ? !!transcriptModelConfig.model?.trim()
        : transcriptModelConfig.provider === uiProvider && modelOptions[uiProvider].includes(transcriptModelConfig.model);
    const canSaveCloudConfig = isModelValidForProvider && (!requiresApiKey || !!apiKey?.trim());

    const handleInputClick = () => {
        if (isApiKeyLocked) {
            setIsLockButtonVibrating(true);
            setTimeout(() => setIsLockButtonVibrating(false), 500);
        }
    };

    const handleWhisperModelSelect = (modelName: string) => {
        // Always update config when model is selected, regardless of current provider
        // This ensures the model is set when user switches back
        setTranscriptModelConfig({
            ...transcriptModelConfig,
            provider: 'localWhisper', // Ensure provider is set correctly
            model: modelName
        });
        // Close modal after selection
        if (onModelSelect) {
            onModelSelect();
        }
    };

    const handleParakeetModelSelect = (modelName: string) => {
        // Always update config when model is selected, regardless of current provider
        // This ensures the model is set when user switches back
        setTranscriptModelConfig({
            ...transcriptModelConfig,
            provider: 'parakeet', // Ensure provider is set correctly
            model: modelName
        });
        // Close modal after selection
        if (onModelSelect) {
            onModelSelect();
        }
    };

    // Persist the currently-selected cloud provider config (provider/model/apiKey/baseUrl).
    // Local providers (localWhisper/parakeet) save themselves via ModelManager/ParakeetModelManager
    // autoSave and never reach this handler.
    const handleSaveCloudConfig = async () => {
        const configToSave: TranscriptModelProps = {
            provider: uiProvider,
            model: transcriptModelConfig.model,
            apiKey: apiKey ?? null,
            baseUrl: uiProvider === 'custom' ? (baseUrl ?? null) : null,
        };

        setIsSaving(true);
        try {
            await onSave?.(configToSave);
            setTranscriptModelConfig(configToSave);
        } finally {
            setIsSaving(false);
        }
    };

    return (
        <div>
            <div>
                {/* <div className="flex justify-between items-center mb-4">
                    <h3 className="text-lg font-semibold text-gray-900">Transcript Settings</h3>
                </div> */}
                <div className="space-y-4 pb-6">
                    <div>
                        <Label className="block text-sm font-medium text-gray-700 mb-1">
                            Transcript Model
                        </Label>
                        <div className="flex space-x-2 mx-1">
                            <Select
                                value={uiProvider}
                                onValueChange={(value) => {
                                    const provider = value as TranscriptModelProps['provider'];
                                    setUiProvider(provider);
                                    if (provider !== 'localWhisper' && provider !== 'parakeet') {
                                        fetchApiKey(provider);
                                    }
                                }}
                            >
                                <SelectTrigger className='focus:ring-1 focus:ring-blue-500 focus:border-blue-500'>
                                    <SelectValue placeholder="Select provider" />
                                </SelectTrigger>
                                <SelectContent>
                                    <SelectItem value="parakeet">⚡ Parakeet (Recommended - Real-time / Accurate)</SelectItem>
                                    <SelectItem value="localWhisper">🏠 Local Whisper (High Accuracy)</SelectItem>
                                    <SelectItem value="openrouter">☁️ OpenRouter</SelectItem>
                                    <SelectItem value="groq">☁️ Groq</SelectItem>
                                    <SelectItem value="openai">☁️ OpenAI</SelectItem>
                                    <SelectItem value="custom">☁️ Custom (OpenAI-compatible)</SelectItem>
                                </SelectContent>
                            </Select>

                            {uiProvider !== 'localWhisper' && uiProvider !== 'parakeet' && (
                                uiProvider === 'custom' ? (
                                    <Input
                                        type="text"
                                        className="focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
                                        value={transcriptModelConfig.model}
                                        onChange={(e) => {
                                            const model = e.target.value;
                                            setTranscriptModelConfig({ ...transcriptModelConfig, provider: uiProvider, model });
                                        }}
                                        placeholder="Model name (e.g. whisper-1)"
                                    />
                                ) : (
                                    <Select
                                        value={transcriptModelConfig.model}
                                        onValueChange={(value) => {
                                            const model = value as TranscriptModelProps['model'];
                                            setTranscriptModelConfig({ ...transcriptModelConfig, provider: uiProvider, model });
                                        }}
                                    >
                                        <SelectTrigger className='focus:ring-1 focus:ring-blue-500 focus:border-blue-500'>
                                            <SelectValue placeholder="Select model" />
                                        </SelectTrigger>
                                        <SelectContent>
                                            {modelOptions[uiProvider].map((model) => (
                                                <SelectItem key={model} value={model}>{model}</SelectItem>
                                            ))}
                                        </SelectContent>
                                    </Select>
                                )
                            )}

                        </div>
                    </div>

                    {uiProvider === 'custom' && (
                        <div>
                            <Label className="block text-sm font-medium text-gray-700 mb-1">
                                Base URL
                            </Label>
                            <Input
                                type="text"
                                className="mx-1 focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
                                value={baseUrl || ''}
                                onChange={(e) => setBaseUrl(e.target.value)}
                                placeholder="https://api.example.com/v1"
                            />
                        </div>
                    )}

                    {isCloudProvider && (
                        <div className="flex items-start gap-2 bg-amber-50 border border-amber-200 rounded p-2 mx-1">
                            <AlertTriangle className="h-4 w-4 text-amber-600 flex-shrink-0 mt-0.5" />
                            <p className="text-xs text-amber-800">
                                Audio is sent to {CLOUD_PROVIDER_LABELS[uiProvider] ?? uiProvider} for transcription. Local Whisper/Parakeet keep recordings on your device.
                            </p>
                        </div>
                    )}

                    {uiProvider === 'localWhisper' && (
                        <div className="mt-6">
                            <ModelManager
                                selectedModel={transcriptModelConfig.provider === 'localWhisper' ? transcriptModelConfig.model : undefined}
                                onModelSelect={handleWhisperModelSelect}
                                autoSave={true}
                            />
                        </div>
                    )}

                    {uiProvider === 'parakeet' && (
                        <div className="mt-6">
                            <ParakeetModelManager
                                selectedModel={transcriptModelConfig.provider === 'parakeet' ? transcriptModelConfig.model : undefined}
                                onModelSelect={handleParakeetModelSelect}
                                autoSave={true}
                            />
                        </div>
                    )}


                    {requiresApiKey && (
                        <div>
                            <Label className="block text-sm font-medium text-gray-700 mb-1">
                                API Key
                            </Label>
                            <div className="relative mx-1">
                                <Input
                                    type={showApiKey ? "text" : "password"}
                                    className={`pr-24 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 ${isApiKeyLocked ? 'bg-gray-100 cursor-not-allowed' : ''
                                        }`}
                                    value={apiKey || ''}
                                    onChange={(e) => setApiKey(e.target.value)}
                                    disabled={isApiKeyLocked}
                                    onClick={handleInputClick}
                                    placeholder="Enter your API key"
                                />
                                {isApiKeyLocked && (
                                    <div
                                        onClick={handleInputClick}
                                        className="absolute inset-0 flex items-center justify-center bg-gray-100 bg-opacity-50 rounded-md cursor-not-allowed"
                                    />
                                )}
                                <div className="absolute inset-y-0 right-0 pr-1 flex items-center">
                                    <Button
                                        type="button"
                                        variant="ghost"
                                        size="icon"
                                        onClick={() => setIsApiKeyLocked(!isApiKeyLocked)}
                                        className={`transition-colors duration-200 ${isLockButtonVibrating ? 'animate-vibrate text-red-500' : ''
                                            }`}
                                        title={isApiKeyLocked ? "Unlock to edit" : "Lock to prevent editing"}
                                    >
                                        {isApiKeyLocked ? <Lock className="h-4 w-4" /> : <Unlock className="h-4 w-4" />}
                                    </Button>
                                    <Button
                                        type="button"
                                        variant="ghost"
                                        size="icon"
                                        onClick={() => setShowApiKey(!showApiKey)}
                                    >
                                        {showApiKey ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                                    </Button>
                                </div>
                            </div>
                        </div>
                    )}

                    {isCloudProvider && (
                        <div className="mx-1">
                            <Button
                                type="button"
                                onClick={handleSaveCloudConfig}
                                disabled={!canSaveCloudConfig || isSaving}
                            >
                                {isSaving ? 'Saving...' : 'Save'}
                            </Button>
                        </div>
                    )}
                </div>
            </div>
        </div >
    )
}








