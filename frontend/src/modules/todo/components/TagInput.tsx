import React, { useState } from 'react';
import { X } from 'lucide-react';

export interface TagInputProps {
  value: string[];
  onChange: (tags: string[]) => void;
  /** Accessible name; also rendered as the visible field label. */
  label?: string;
}

/** Close enough to the server normalization to dedupe chips; the server stays authoritative. */
const normalize = (raw: string) => raw.trim().replace(/^#/, '').toLowerCase();

/**
 * Chip input for tags. `Enter` or `,` commits the draft, `Backspace` on an
 * empty draft removes the last chip. Validation is the server job, so a bad
 * tag surfaces as the request error rather than being second-guessed here.
 */
export const TagInput: React.FC<TagInputProps> = ({ value, onChange, label = 'Tags' }) => {
  const [draft, setDraft] = useState('');

  const commit = (raw: string) => {
    const tag = normalize(raw);
    setDraft('');
    if (tag && !value.includes(tag)) onChange([...value, tag]);
  };

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const next = e.target.value;
    if (next.includes(',')) {
      const parts = next.split(',');
      const rest = parts.pop() ?? '';
      const added = parts.map(normalize).filter((t) => t && !value.includes(t));
      onChange([...value, ...new Set(added)]);
      setDraft(rest);
    } else {
      setDraft(next);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      // A draft is consumed here; an empty Enter falls through and submits the form.
      if (draft.trim()) {
        e.preventDefault();
        commit(draft);
      }
    } else if (e.key === 'Backspace' && draft === '' && value.length > 0) {
      onChange(value.slice(0, -1));
    }
  };

  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">{label}</span>
      <div className="flex flex-wrap items-center gap-1.5 min-h-9 px-2 py-1 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 focus-within:border-neutral-500 transition-colors duration-150">
        {value.map((tag) => (
          <span
            key={tag}
            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-xs font-mono text-neutral-700 dark:text-neutral-300"
          >
            #{tag}
            <button
              type="button"
              aria-label={`Remove tag ${tag}`}
              onClick={() => onChange(value.filter((t) => t !== tag))}
              className="text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 transition-colors duration-150 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-neutral-500"
            >
              <X className="w-3 h-3" />
            </button>
          </span>
        ))}
        <input
          type="text"
          aria-label={label}
          value={draft}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          onBlur={() => draft.trim() && commit(draft)}
          placeholder={value.length === 0 ? 'Add a tag' : ''}
          className="flex-1 min-w-24 bg-transparent text-xs font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none"
        />
      </div>
    </div>
  );
};
