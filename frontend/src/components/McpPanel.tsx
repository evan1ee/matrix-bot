import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Check, Copy, Key, Loader2, Plus, Terminal, Trash2 } from 'lucide-react';
import { fetchApi } from '../core/apiClient';
import { copyToClipboard } from '../core/clipboard';
import { formatDateInZone, formatDateTimeInZone, useSettings } from '../core/settings';

interface McpTokenSummary {
  id: string;
  name: string;
  prefix: string;
  created_at: string;
  last_used_at: string | null;
  expires_at: string | null;
}

interface CreateMcpTokenResponse {
  id: string;
  name: string;
  token: string;
  prefix: string;
  created_at: string;
  expires_at: string | null;
  claude_config: Record<string, any>;
  cursor_config: Record<string, any>;
}

export const McpPanel: React.FC = () => {
  const queryClient = useQueryClient();
  const settings = useSettings();
  const [tokenName, setTokenName] = useState('Claude Desktop');
  const [createdToken, setCreatedToken] = useState<CreateMcpTokenResponse | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'claude' | 'cursor' | 'raw'>('claude');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { data: tokens = [], isLoading } = useQuery<McpTokenSummary[]>({
    queryKey: ['mcp-tokens'],
    queryFn: () => fetchApi<McpTokenSummary[]>('/mcp/tokens'),
  });

  const createMutation = useMutation({
    mutationFn: (name: string) =>
      fetchApi<CreateMcpTokenResponse>('/mcp/tokens', {
        method: 'POST',
        body: JSON.stringify({ name }),
      }),
    onSuccess: (data) => {
      setCreatedToken(data);
      setErrorMsg(null);
      queryClient.invalidateQueries({ queryKey: ['mcp-tokens'] });
    },
    onError: (err: any) => {
      setErrorMsg(err?.message || 'Failed to create token');
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      fetchApi(`/mcp/tokens/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['mcp-tokens'] });
    },
  });

  const handleCopy = async (key: string, text: string) => {
    const ok = await copyToClipboard(text);
    if (ok) {
      setCopiedKey(key);
      setTimeout(() => setCopiedKey(null), 2000);
    }
  };

  const origin = typeof window !== 'undefined' ? window.location.origin : '';
  const sseUrl = `${origin}/api/v1/mcp/sse`;

  const getClaudeSnippet = (tokenStr: string) => {
    return JSON.stringify(
      {
        mcpServers: {
          'matrix-bot': {
            url: sseUrl,
            headers: {
              Authorization: `Bearer ${tokenStr}`,
            },
          },
        },
      },
      null,
      2
    );
  };

  const getCursorSnippet = (tokenStr: string) => {
    return JSON.stringify(
      {
        mcpServers: {
          'matrix-bot': {
            url: sseUrl,
            headers: {
              Authorization: `Bearer ${tokenStr}`,
            },
          },
        },
      },
      null,
      2
    );
  };

  return (
    <div className="space-y-6">
      {/* Intro Header */}
      <div className="space-y-1">
        <h4 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 flex items-center gap-2">
          <Key className="w-4 h-4" />
          Model Context Protocol (MCP)
        </h4>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          Connect external AI environments like <b>Claude Desktop</b> and <b>Cursor</b> directly to
          your Matrix Bot tools and live data.
        </p>
      </div>

      {errorMsg && (
        <div className="p-3 rounded bg-rose-100 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-800/50 text-rose-800 dark:text-rose-300 text-xs font-mono">
          {errorMsg}
        </div>
      )}

      {/* Token Created Modal / Banner */}
      {createdToken && (
        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-900/60 border border-neutral-300 dark:border-neutral-700 space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-emerald-700 dark:text-emerald-400 flex items-center gap-1.5">
              <Check className="w-3.5 h-3.5" />
              Token Generated: {createdToken.name}
            </span>
            <button
              type="button"
              onClick={() => setCreatedToken(null)}
              className="text-xs text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 transition"
            >
              Done
            </button>
          </div>

          <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
            Copy your token or ready-to-use configuration now. The raw token will not be displayed again.
          </p>

          {/* Config Format Tabs */}
          <div className="flex items-center gap-2 border-b border-neutral-200 dark:border-neutral-800 pb-2 text-xs">
            <button
              type="button"
              onClick={() => setActiveTab('claude')}
              className={`font-mono px-2 py-0.5 rounded transition ${
                activeTab === 'claude'
                  ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              Claude Desktop
            </button>
            <button
              type="button"
              onClick={() => setActiveTab('cursor')}
              className={`font-mono px-2 py-0.5 rounded transition ${
                activeTab === 'cursor'
                  ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              Cursor
            </button>
            <button
              type="button"
              onClick={() => setActiveTab('raw')}
              className={`font-mono px-2 py-0.5 rounded transition ${
                activeTab === 'raw'
                  ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              Raw Token
            </button>
          </div>

          {activeTab === 'raw' ? (
            <div className="flex items-center gap-2">
              <input
                type="text"
                readOnly
                value={createdToken.token}
                className="flex-1 bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded px-3 py-1.5 text-xs font-mono text-neutral-800 dark:text-neutral-200 select-all"
              />
              <button
                type="button"
                onClick={() => handleCopy('token', createdToken.token)}
                className="px-3 py-1.5 bg-neutral-900 hover:bg-neutral-800 dark:bg-neutral-100 dark:hover:bg-neutral-200 text-white dark:text-neutral-900 rounded text-xs font-medium flex items-center gap-1.5 transition"
              >
                {copiedKey === 'token' ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
                {copiedKey === 'token' ? 'Copied' : 'Copy'}
              </button>
            </div>
          ) : (
            <div className="space-y-2">
              <div className="relative">
                <pre className="p-3 rounded bg-neutral-100 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 text-[11px] font-mono text-neutral-800 dark:text-neutral-200 overflow-x-auto max-h-48">
                  {activeTab === 'claude'
                    ? getClaudeSnippet(createdToken.token)
                    : getCursorSnippet(createdToken.token)}
                </pre>
                <button
                  type="button"
                  onClick={() =>
                    handleCopy(
                      'snippet',
                      activeTab === 'claude'
                        ? getClaudeSnippet(createdToken.token)
                        : getCursorSnippet(createdToken.token)
                    )
                  }
                  className="absolute right-2 top-2 px-2.5 py-1 bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-white rounded text-xs font-medium flex items-center gap-1 shadow-sm transition"
                >
                  {copiedKey === 'snippet' ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                  {copiedKey === 'snippet' ? 'Copied' : 'Copy JSON'}
                </button>
              </div>
              <p className="text-[10px] text-neutral-400">
                {activeTab === 'claude'
                  ? 'Paste into ~/Library/Application Support/Claude/claude_desktop_config.json'
                  : 'Paste into .cursor/mcp.json or Cursor Settings → Features → MCP'}
              </p>
            </div>
          )}
        </div>
      )}

      {/* New Token Generator Form */}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (tokenName.trim()) {
            createMutation.mutate(tokenName.trim());
          }
        }}
        className="flex items-center gap-2"
      >
        <input
          type="text"
          value={tokenName}
          onChange={(e) => setTokenName(e.target.value)}
          placeholder="Connection name, e.g. Claude Desktop"
          maxLength={100}
          className="flex-1 bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded-lg px-3 py-2 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition"
        />
        <button
          type="submit"
          disabled={createMutation.isPending || !tokenName.trim()}
          className="inline-flex items-center gap-1.5 px-3 py-2 bg-neutral-900 hover:bg-neutral-800 dark:bg-neutral-100 dark:hover:bg-neutral-200 text-white dark:text-neutral-900 rounded-lg text-xs font-medium transition disabled:opacity-50"
        >
          {createMutation.isPending ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Plus className="w-3.5 h-3.5" />
          )}
          Generate Token
        </button>
      </form>

      {/* Active Tokens List */}
      <div className="space-y-3 pt-2">
        <h5 className="text-xs font-mono uppercase tracking-wider text-neutral-400">
          Active Connections ({tokens.length})
        </h5>

        {isLoading ? (
          <div className="py-4 text-center text-xs text-neutral-400 flex items-center justify-center gap-2">
            <Loader2 className="w-3.5 h-3.5 animate-spin" /> Loading connections...
          </div>
        ) : tokens.length === 0 ? (
          <div className="p-4 rounded-lg border border-dashed border-neutral-200 dark:border-neutral-800 text-center text-xs text-neutral-400">
            No active MCP connections. Generate a token above or type <code className="font-mono text-neutral-700 dark:text-neutral-300">!mcp token</code> in Matrix DM.
          </div>
        ) : (
          <div className="divide-y divide-neutral-100 dark:divide-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-lg overflow-hidden">
            {tokens.map((tok) => (
              <div
                key={tok.id}
                className="p-3 bg-white dark:bg-neutral-950 flex items-center justify-between gap-3 text-xs"
              >
                <div className="space-y-0.5 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-neutral-900 dark:text-neutral-100 truncate">
                      {tok.name}
                    </span>
                    <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-neutral-100 dark:bg-neutral-900 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-800">
                      {tok.prefix}
                    </span>
                  </div>
                  <div className="text-[11px] text-neutral-400 flex items-center gap-3">
                    <span>
                      Created: {formatDateInZone(tok.created_at, settings.timezone)}
                    </span>
                    <span>•</span>
                    <span>
                      Last active:{' '}
                      {tok.last_used_at
                        ? formatDateTimeInZone(tok.last_used_at, settings.timezone)
                        : 'Never'}
                    </span>
                  </div>
                </div>

                <button
                  type="button"
                  onClick={() => deleteMutation.mutate(tok.id)}
                  disabled={deleteMutation.isPending}
                  aria-label={`Revoke ${tok.name}`}
                  className="p-1.5 text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 rounded hover:bg-neutral-100 dark:hover:bg-neutral-900 transition"
                  title="Revoke Token"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* In-Chat Alternative Tip */}
      <div className="p-3 rounded-lg bg-neutral-50 dark:bg-neutral-900/40 border border-neutral-200 dark:border-neutral-800/80 text-[11px] text-neutral-500 dark:text-neutral-400 flex items-start gap-2">
        <Terminal className="w-4 h-4 text-neutral-400 shrink-0 mt-0.5" />
        <div>
          <b>Chat Shortcut:</b> You can also generate tokens directly inside Matrix. Just send{' '}
          <code className="font-mono text-neutral-800 dark:text-neutral-200">!mcp token [name]</code> in a private direct message with the bot.
        </div>
      </div>
    </div>
  );
};
