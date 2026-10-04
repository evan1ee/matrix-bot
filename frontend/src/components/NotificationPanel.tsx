import React, { useState } from 'react';
import {
  Bell,
  Check,
  Globe,
  Loader2,
  Mail,
  MessageSquare,
  Radio,
  Send,
  Shield,
  Smartphone,
} from 'lucide-react';
import { channelLabel, useIdentities } from '../core/security';
import { useSettings, useUpdateSettings } from '../core/settings';

export const NotificationPanel: React.FC = () => {
  const { data: identities = [], isLoading: isLoadingIdentities } = useIdentities();
  const settings = useSettings();
  const updateSettings = useUpdateSettings();

  const [savedSuccess, setSavedSuccess] = useState(false);

  // Preferred notification channel routing (saved locally in browser or syncs with settings)
  const [briefingTarget, setBriefingTarget] = useState<string>(() => {
    return localStorage.getItem('matrixbot:routing:briefing') || 'default';
  });
  const [remindersTarget, setRemindersTarget] = useState<string>(() => {
    return localStorage.getItem('matrixbot:routing:reminders') || 'origin';
  });
  const [agentTarget, setAgentTarget] = useState<string>(() => {
    return localStorage.getItem('matrixbot:routing:agent') || 'all';
  });

  const handleSave = () => {
    localStorage.setItem('matrixbot:routing:briefing', briefingTarget);
    localStorage.setItem('matrixbot:routing:reminders', remindersTarget);
    localStorage.setItem('matrixbot:routing:agent', agentTarget);

    setSavedSuccess(true);
    setTimeout(() => setSavedSuccess(false), 2000);
  };

  const getChannelIcon = (channel: string) => {
    switch (channel.toLowerCase()) {
      case 'matrix':
        return <MessageSquare size={14} className="text-neutral-700 dark:text-neutral-300" />;
      case 'discord':
        return <Radio size={14} className="text-indigo-500" />;
      case 'telegram':
        return <Send size={14} className="text-sky-500" />;
      case 'email':
        return <Mail size={14} className="text-amber-500" />;
      default:
        return <Globe size={14} className="text-neutral-500" />;
    }
  };

  return (
    <div className="space-y-6 text-xs font-sans text-neutral-900 dark:text-neutral-100">
      {/* Connected Channels Summary */}
      <section className="space-y-3">
        <div>
          <h4 className="font-semibold text-sm">Connected Channels</h4>
          <p className="text-neutral-500 dark:text-neutral-400 text-xs">
            Platforms authorized to receive bot notifications and alerts.
          </p>
        </div>

        {isLoadingIdentities ? (
          <div className="flex items-center gap-2 py-4 text-neutral-400">
            <Loader2 size={14} className="animate-spin" />
            <span>Loading connected channels...</span>
          </div>
        ) : identities.length === 0 ? (
          <div className="p-4 rounded-xl border border-dashed border-neutral-200 dark:border-neutral-800 text-center text-neutral-500 space-y-1">
            <p>No chat identities linked yet.</p>
            <p className="text-[11px] text-neutral-400">
              Send <code className="font-mono bg-neutral-100 dark:bg-neutral-800 px-1 py-0.5 rounded">!link</code> in Matrix, Discord, Telegram, or Email to connect your account.
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
            {identities.map((id, idx) => (
              <div
                key={`${id.channel}-${id.user_id}`}
                className="flex items-center justify-between p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-950/40"
              >
                <div className="flex items-center gap-2.5 min-w-0">
                  <div className="p-1.5 rounded-md bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shrink-0">
                    {getChannelIcon(id.channel)}
                  </div>
                  <div className="min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="font-medium text-xs truncate">
                        {channelLabel(id.channel)}
                      </span>
                      {idx === 0 && (
                        <span className="text-[9px] font-mono px-1 py-0.2 bg-neutral-200 dark:bg-neutral-800 rounded text-neutral-600 dark:text-neutral-400">
                          primary
                        </span>
                      )}
                    </div>
                    <span className="font-mono text-[10px] text-neutral-400 truncate block">
                      {id.user_id}
                    </span>
                  </div>
                </div>
                <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium bg-emerald-100 dark:bg-emerald-950/60 text-emerald-800 dark:text-emerald-300 shrink-0">
                  Ready
                </span>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Notification Category Routing */}
      <section className="space-y-4 pt-4 border-t border-neutral-200 dark:border-neutral-800">
        <div>
          <h4 className="font-semibold text-sm">Delivery Preferences</h4>
          <p className="text-neutral-500 dark:text-neutral-400 text-xs">
            Configure which destination channels receive specific notifications.
          </p>
        </div>

        <div className="space-y-3">
          {/* Morning Briefing Routing */}
          <div className="p-3.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900/60 flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div>
              <span className="font-medium block text-neutral-900 dark:text-neutral-100">
                Daily Morning Briefing
              </span>
              <span className="text-[11px] text-neutral-500 block">
                Scheduled delivery of agenda, weather, and reminders.
              </span>
            </div>
            <select
              value={briefingTarget}
              onChange={(e) => setBriefingTarget(e.target.value)}
              className="px-2.5 py-1.5 rounded-md bg-neutral-50 dark:bg-neutral-950 border border-neutral-300 dark:border-neutral-700 text-xs font-mono"
            >
              <option value="default">Default (Primary Channel)</option>
              {identities.map((id) => (
                <option key={`briefing-${id.channel}`} value={id.channel}>
                  {channelLabel(id.channel)} ({id.user_id})
                </option>
              ))}
              <option value="web">Web Only</option>
            </select>
          </div>

          {/* Reminders Routing */}
          <div className="p-3.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900/60 flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div>
              <span className="font-medium block text-neutral-900 dark:text-neutral-100">
                Task & Event Reminders
              </span>
              <span className="text-[11px] text-neutral-500 block">
                Timed reminder alerts when tasks or recurring bills become due.
              </span>
            </div>
            <select
              value={remindersTarget}
              onChange={(e) => setRemindersTarget(e.target.value)}
              className="px-2.5 py-1.5 rounded-md bg-neutral-50 dark:bg-neutral-950 border border-neutral-300 dark:border-neutral-700 text-xs font-mono"
            >
              <option value="origin">Origin Conversation (Where created)</option>
              <option value="all">Broadcast to All Connected Channels</option>
              <option value="primary">Primary Channel Only</option>
            </select>
          </div>

          {/* Agent Approvals Routing */}
          <div className="p-3.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900/60 flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div>
              <span className="font-medium block text-neutral-900 dark:text-neutral-100">
                Autonomous Agent Alerts & Approvals
              </span>
              <span className="text-[11px] text-neutral-500 block">
                Interactive requests when the agent requests authorization for sensitive actions.
              </span>
            </div>
            <select
              value={agentTarget}
              onChange={(e) => setAgentTarget(e.target.value)}
              className="px-2.5 py-1.5 rounded-md bg-neutral-50 dark:bg-neutral-950 border border-neutral-300 dark:border-neutral-700 text-xs font-mono"
            >
              <option value="all">Every Connected Channel & Web</option>
              <option value="primary">Primary Chat Channel</option>
              <option value="web">Web Console Only</option>
            </select>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 pt-2">
          {savedSuccess && (
            <span className="text-emerald-600 dark:text-emerald-400 font-medium inline-flex items-center gap-1">
              <Check size={13} />
              Routing preferences saved
            </span>
          )}
          <button
            type="button"
            onClick={handleSave}
            className="px-4 py-2 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors cursor-pointer"
          >
            Save Routing Preferences
          </button>
        </div>
      </section>
    </div>
  );
};
