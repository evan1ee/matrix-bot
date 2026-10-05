import React from 'react';
import { Tag } from 'lucide-react';

interface TagFilterChipsProps {
  tags: string[];
  selectedTag: string | null;
  onSelectTag: (tag: string | null) => void;
}

export const TagFilterChips: React.FC<TagFilterChipsProps> = ({
  tags,
  selectedTag,
  onSelectTag,
}) => {
  if (tags.length === 0) return null;

  return (
    <div data-testid="tag-filter-chips" className="flex items-center gap-1.5 overflow-x-auto py-1 scrollbar-none">
      <div className="flex items-center text-xs text-neutral-400 mr-1 shrink-0">
        <Tag className="w-3.5 h-3.5 mr-1" />
        <span>Tags:</span>
      </div>
      <button
        type="button"
        onClick={() => onSelectTag(null)}
        className={`px-2.5 py-1 text-xs rounded-md transition-colors shrink-0 font-mono ${
          selectedTag === null
            ? 'bg-neutral-900 text-neutral-50 dark:bg-neutral-100 dark:text-neutral-900 font-medium'
            : 'bg-neutral-100 text-neutral-600 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-700'
        }`}
      >
        All
      </button>
      {tags.map((tag) => {
        const isSelected = selectedTag === tag;
        return (
          <button
            key={tag}
            type="button"
            onClick={() => onSelectTag(isSelected ? null : tag)}
            className={`px-2.5 py-1 text-xs rounded-md transition-colors shrink-0 font-mono ${
              isSelected
                ? 'bg-neutral-900 text-neutral-50 dark:bg-neutral-100 dark:text-neutral-900 font-medium'
                : 'bg-neutral-100 text-neutral-600 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-700'
            }`}
          >
            #{tag}
          </button>
        );
      })}
    </div>
  );
};
