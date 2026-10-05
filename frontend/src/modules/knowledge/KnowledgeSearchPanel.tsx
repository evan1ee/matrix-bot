import React, { useState } from 'react';
import { Search, Sparkles, FileText, Bookmark, ExternalLink } from 'lucide-react';
import { useKnowledgeSearch } from './api';
import type { SearchResultItem } from './types';

interface KnowledgeSearchPanelProps {
  onSelectResult?: (item: SearchResultItem) => void;
}

export const KnowledgeSearchPanel: React.FC<KnowledgeSearchPanelProps> = ({
  onSelectResult,
}) => {
  const [query, setQuery] = useState('');
  const [selectedType, setSelectedType] = useState<string>('all');

  const filterTypes = selectedType === 'all' ? undefined : [selectedType];
  const { data: results = [], isLoading, isFetching } = useKnowledgeSearch(
    query,
    filterTypes,
    8
  );

  return (
    <div className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-xl p-4 shadow-xs space-y-3">
      {/* Search Input and Type Filters */}
      <div className="flex flex-col sm:flex-row gap-2">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-neutral-400" />
          <input
            type="text"
            placeholder="Semantic & hybrid search across notes, bookmarks..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="w-full pl-9 pr-4 py-2 text-sm bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 rounded-md text-neutral-900 dark:text-neutral-100 placeholder:text-neutral-400 focus:outline-hidden focus:ring-2 focus:ring-neutral-400 dark:focus:ring-neutral-600 transition"
          />
        </div>

        {/* Source Filter Chips */}
        <div className="flex items-center gap-1 shrink-0 overflow-x-auto">
          {[
            { id: 'all', label: 'All' },
            { id: 'note', label: 'Notes' },
            { id: 'bookmark', label: 'Bookmarks' },
          ].map((type) => (
            <button
              key={type.id}
              type="button"
              onClick={() => setSelectedType(type.id)}
              className={`px-3 py-1.5 text-xs rounded-md font-medium transition-colors ${
                selectedType === type.id
                  ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'
                  : 'bg-neutral-100 text-neutral-600 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-700'
              }`}
            >
              {type.label}
            </button>
          ))}
        </div>
      </div>

      {/* Results View */}
      {query.trim().length > 0 && (
        <div className="pt-2 border-t border-neutral-100 dark:border-neutral-800 space-y-2">
          <div className="flex items-center justify-between text-xs text-neutral-400 px-1">
            <span className="flex items-center gap-1 font-mono">
              <Sparkles className="w-3.5 h-3.5" />
              {isFetching ? 'Searching embeddings...' : `${results.length} results`}
            </span>
            <span className="font-mono text-[10px]">hybrid vector + text</span>
          </div>

          {isLoading ? (
            <div className="py-6 text-center text-xs text-neutral-400 font-mono">
              Computing query embeddings...
            </div>
          ) : results.length === 0 ? (
            <div className="py-6 text-center text-xs text-neutral-400">
              No matching knowledge items found for "{query}"
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-2 max-h-[360px] overflow-y-auto pr-1">
              {results.map((item, idx) => {
                const scorePercent = Math.round(item.similarity_score * 100);
                const isNote = item.item_type === 'note';

                return (
                  <div
                    key={`${item.item_type}-${item.item_id}-${idx}`}
                    onClick={() => onSelectResult?.(item)}
                    className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 hover:border-neutral-400 dark:hover:border-neutral-600 hover:bg-neutral-50 dark:hover:bg-neutral-800/50 transition cursor-pointer text-left group"
                  >
                    <div className="flex items-start justify-between gap-2 mb-1">
                      <div className="flex items-center gap-1.5 min-w-0">
                        {isNote ? (
                          <FileText className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
                        ) : (
                          <Bookmark className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
                        )}
                        <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 truncate group-hover:text-blue-600 dark:group-hover:text-blue-400 transition-colors">
                          {item.title}
                        </span>
                      </div>
                      <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 shrink-0">
                        {scorePercent}% match
                      </span>
                    </div>

                    <p className="text-xs text-neutral-600 dark:text-neutral-400 line-clamp-2 leading-relaxed">
                      {item.content_snippet}
                    </p>

                    {item.source_ref && (
                      <div className="mt-1.5 flex items-center gap-1 text-[11px] text-neutral-400 truncate font-mono">
                        <ExternalLink className="w-3 h-3 shrink-0" />
                        <span className="truncate">{item.source_ref}</span>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
};
