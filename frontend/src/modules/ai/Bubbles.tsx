import React from 'react';
import { Bot, Clock, User } from 'lucide-react';
import { MarkdownRenderer } from '../../components/MarkdownRenderer';

export const AssistantRow: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="flex gap-3 max-w-3xl mr-auto justify-start">
    <div className="w-7 h-7 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 flex items-center justify-center shrink-0 mt-0.5">
      <Bot className="w-3.5 h-3.5" />
    </div>
    {children}
  </div>
);

export const UserBubble: React.FC<{ content: string; time?: string }> = ({ content, time }) => (
  <div className="flex gap-3 max-w-3xl ml-auto justify-end">
    <div className="p-3.5 rounded-lg text-sm leading-relaxed space-y-2 max-w-2xl font-sans bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 shadow-xs">
      <MarkdownRenderer content={content} />
      {time && (
        <div className="text-[10px] font-mono flex items-center gap-1 pt-1 text-neutral-300 dark:text-neutral-600 justify-end">
          <Clock className="w-2.5 h-2.5" />
          <span>{time}</span>
        </div>
      )}
    </div>
    <div className="w-7 h-7 rounded-full bg-neutral-200 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 flex items-center justify-center shrink-0 mt-0.5 font-mono text-[11px]">
      <User className="w-3.5 h-3.5" />
    </div>
  </div>
);
