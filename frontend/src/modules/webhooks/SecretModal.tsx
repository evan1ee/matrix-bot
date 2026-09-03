import React, { useState } from 'react';
import { ShieldCheck, AlertTriangle, Copy, Check, Terminal } from 'lucide-react';
import { CreatedWebhookResponse } from './types';

interface SecretModalProps {
  data: CreatedWebhookResponse;
  onClose: () => void;
}

export const SecretModal: React.FC<SecretModalProps> = ({ data, onClose }) => {
  const [copiedType, setCopiedType] = useState<string | null>(null);

  const handleCopy = (text: string, type: string) => {
    navigator.clipboard.writeText(text);
    setCopiedType(type);
    setTimeout(() => setCopiedType(null), 2000);
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
    >
      <div className="bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 w-full max-w-2xl rounded-xl border border-neutral-200 dark:border-neutral-800 shadow-2xl overflow-hidden animate-in fade-in-0 zoom-in-95 max-h-[90vh] flex flex-col">
        <div className="p-6 border-b border-neutral-200 dark:border-neutral-800 bg-emerald-500/5">
          <div className="flex items-center gap-2.5 text-emerald-600 dark:text-emerald-400 font-semibold text-base">
            <ShieldCheck className="w-5 h-5" />
            Webhook Secret Generated
          </div>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
            Your webhook <strong>{data.webhook.name}</strong> is ready.
          </p>
        </div>

        <div className="p-6 overflow-y-auto space-y-5">
          <div className="p-3.5 bg-amber-500/10 border border-amber-500/20 rounded-lg flex items-start gap-2.5 text-amber-700 dark:text-amber-400 text-xs">
            <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
            <div>
              <strong>Copy this secret token now.</strong> It is stored securely as a SHA-256 hash and cannot be viewed again once this dialog is closed.
            </div>
          </div>

          <div>
            <label className="block text-xs font-semibold text-neutral-700 dark:text-neutral-300 mb-1.5 uppercase tracking-wider">
              Secret Token
            </label>
            <div className="flex items-center gap-2">
              <input
                type="text"
                readOnly
                value={data.raw_token}
                className="w-full font-mono text-xs px-3 py-2.5 bg-neutral-100 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 rounded-lg text-neutral-900 dark:text-neutral-100 select-all"
              />
              <button
                onClick={() => handleCopy(data.raw_token, 'token')}
                className="inline-flex items-center gap-1.5 px-3 py-2.5 text-xs font-medium bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 rounded-lg transition-colors shrink-0 cursor-pointer"
              >
                {copiedType === 'token' ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
                {copiedType === 'token' ? 'Copied' : 'Copy'}
              </button>
            </div>
          </div>

          {/* Sample Commands */}
          <div className="space-y-3 pt-2">
            <div className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 flex items-center gap-1.5 uppercase tracking-wider">
              <Terminal className="w-3.5 h-3.5 text-blue-600 dark:text-blue-400" />
              Example Integrations
            </div>

            <div className="space-y-1.5">
              <div className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400 flex justify-between">
                <span>1. Standard JSON Webhook (Alertmanager / Cron / Python)</span>
                <button
                  onClick={() => handleCopy(data.example_curl_json, 'json')}
                  className="text-blue-600 dark:text-blue-400 hover:underline text-[11px] cursor-pointer"
                >
                  {copiedType === 'json' ? 'Copied' : 'Copy curl'}
                </button>
              </div>
              <pre className="p-3 bg-neutral-950 text-neutral-100 rounded-lg text-xs font-mono overflow-x-auto whitespace-pre-wrap border border-neutral-800">
                {data.example_curl_json}
              </pre>
            </div>

            <div className="space-y-1.5 pt-1">
              <div className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400 flex justify-between">
                <span>2. Telegram Bot API Drop-in Emulation</span>
                <button
                  onClick={() => handleCopy(data.example_curl_telegram, 'tg')}
                  className="text-blue-600 dark:text-blue-400 hover:underline text-[11px] cursor-pointer"
                >
                  {copiedType === 'tg' ? 'Copied' : 'Copy curl'}
                </button>
              </div>
              <pre className="p-3 bg-neutral-950 text-neutral-100 rounded-lg text-xs font-mono overflow-x-auto whitespace-pre-wrap border border-neutral-800">
                {data.example_curl_telegram}
              </pre>
            </div>
          </div>
        </div>

        <div className="p-4 border-t border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900/50 flex justify-end">
          <button
            onClick={onClose}
            className="px-5 py-2 text-sm font-medium bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 rounded-lg transition-colors cursor-pointer"
          >
            I Have Saved My Secret
          </button>
        </div>
      </div>
    </div>
  );
};
