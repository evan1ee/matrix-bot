import React, { useState } from 'react';
import { Footprint, FootprintsSummaryStats } from './types';
import {
  MapPin,
  Calendar,
  Star,
  Trash2,
  Pencil,
  Search,
  Plus,
  Compass,
  Sparkles,
} from 'lucide-react';
import { triggerAiPrompt } from '../ai';

interface FootprintsListProps {
  footprints: Footprint[];
  summaryStats: FootprintsSummaryStats | null;
  selectedFootprint: Footprint | null;
  onSelectFootprint: (fp: Footprint) => void;
  onEditFootprint: (fp: Footprint) => void;
  onDeleteFootprint: (id: number) => void;
  onOpenAddModal: () => void;
}

const CATEGORY_FILTERS = [
  { id: 'all', label: 'All' },
  { id: 'sightseeing', label: 'Sightseeing' },
  { id: 'food', label: 'Food' },
  { id: 'nature', label: 'Nature' },
  { id: 'lodging', label: 'Lodging' },
  { id: 'roadtrip', label: 'Road Trip' },
  { id: 'work', label: 'Work' },
];

export const FootprintsList: React.FC<FootprintsListProps> = ({
  footprints,
  selectedFootprint,
  onSelectFootprint,
  onEditFootprint,
  onDeleteFootprint,
  onOpenAddModal,
}) => {
  const [search, setSearch] = useState('');
  const [activeCategory, setActiveCategory] = useState('all');

  const filtered = footprints.filter((fp) => {
    const matchesCat = activeCategory === 'all' || fp.category === activeCategory;
    const query = search.toLowerCase().trim();
    if (!query) return matchesCat;

    const matchesSearch =
      fp.title.toLowerCase().includes(query) ||
      (fp.city && fp.city.toLowerCase().includes(query)) ||
      (fp.country && fp.country.toLowerCase().includes(query)) ||
      (fp.note && fp.note.toLowerCase().includes(query));

    return matchesCat && matchesSearch;
  });

  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      {/* Top Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3 shrink-0">
        <div className="flex items-center gap-2">
          <Compass className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Visited Places Journal</h3>
        </div>
        <span className="text-xs font-mono text-neutral-500">
          {filtered.length} {filtered.length === 1 ? 'place' : 'places'}
        </span>
      </div>

      {/* Search & Add Action Bar */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div className="relative flex-1 max-w-md">
          <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-neutral-400" />
          <input
            type="text"
            placeholder="Search places, cities, memories..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full h-9 pl-9 pr-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
          />
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <button
            type="button"
            onClick={() =>
              triggerAiPrompt(
                'Review my travel footprints and visited locations. Write a brief narrative summary of my travel journey and suggest future travel ideas.'
              )
            }
            className="h-9 inline-flex items-center gap-1.5 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 text-xs font-medium hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer shadow-2xs"
            title="Ask AI travel log"
          >
            <Sparkles className="w-3.5 h-3.5 text-neutral-500" />
            <span>AI Travel Log</span>
          </button>

          <button
            onClick={onOpenAddModal}
            className="h-9 inline-flex items-center justify-center gap-1.5 px-3.5 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition shrink-0 cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>Add Place</span>
          </button>
        </div>
      </div>

      {/* Category Filter Tabs */}
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-xs font-mono text-neutral-500 dark:text-neutral-400 whitespace-nowrap">CATEGORY</span>
        <div className="flex items-center flex-wrap gap-1 bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-1 text-xs font-mono">
          {CATEGORY_FILTERS.map((cat) => (
            <button
              key={cat.id}
              onClick={() => setActiveCategory(cat.id)}
              className={`px-2.5 py-1 rounded capitalize transition whitespace-nowrap cursor-pointer ${
                activeCategory === cat.id
                  ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                  : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              {cat.label}
            </button>
          ))}
        </div>
      </div>

      {/* Places Grid View */}
      {filtered.length === 0 ? (
        <div className="py-12 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded-lg flex flex-col items-center gap-2">
          <span>No footprints found.</span>
          <button
            onClick={onOpenAddModal}
            className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition cursor-pointer"
          >
            <Plus className="w-3 h-3" />
            <span>Record First Place</span>
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 pt-1">
          {filtered.map((fp) => {
            const isSelected = selectedFootprint?.id === fp.id;
            const formattedSpend =
              fp.spend_amount_minor > 0
                ? `${(fp.spend_amount_minor / Math.pow(10, fp.spend_currency_exponent)).toFixed(2)} ${fp.spend_currency}`
                : null;

            return (
              <div
                key={fp.id}
                onClick={() => onSelectFootprint(fp)}
                className={`rounded-lg border p-4 transition flex flex-col justify-between cursor-pointer bg-white dark:bg-neutral-900 ${
                  isSelected
                    ? 'border-neutral-400 dark:border-neutral-500 ring-1 ring-neutral-400 dark:ring-neutral-500 shadow-sm'
                    : 'border-neutral-200 dark:border-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700'
                }`}
              >
                <div>
                  {/* Title, Stars, and Actions Header */}
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex-1 min-w-0">
                      <h4 className="font-medium text-sm text-neutral-900 dark:text-neutral-100 truncate">
                        {fp.title}
                      </h4>
                      <div className="flex items-center text-amber-500 dark:text-amber-400 mt-0.5">
                        {Array.from({ length: fp.rating }).map((_, i) => (
                          <Star key={i} className="w-3 h-3 fill-amber-500 dark:fill-amber-400" />
                        ))}
                      </div>
                    </div>

                    <div className="flex items-center gap-1 shrink-0" onClick={(e) => e.stopPropagation()}>
                      <button
                        onClick={() => onEditFootprint(fp)}
                        title="Edit Footprint"
                        className="p-1 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 transition cursor-pointer"
                      >
                        <Pencil className="w-3.5 h-3.5" />
                      </button>
                      <button
                        onClick={() => onDeleteFootprint(fp.id)}
                        title="Delete Footprint"
                        className="p-1 rounded text-neutral-400 hover:text-red-600 dark:hover:text-red-400 transition cursor-pointer"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>

                  {/* Location & Dates */}
                  <div className="space-y-1 mt-2.5 text-xs text-neutral-500 font-sans">
                    {(fp.city || fp.country) && (
                      <div className="flex items-center gap-1.5">
                        <MapPin className="w-3.5 h-3.5 text-neutral-400 shrink-0" />
                        <span className="truncate">{[fp.city, fp.country].filter(Boolean).join(', ')}</span>
                      </div>
                    )}

                    <div className="flex items-center gap-1.5 font-mono text-[11px]">
                      <Calendar className="w-3.5 h-3.5 text-neutral-400 shrink-0" />
                      <span>
                        {fp.arrival_date}
                        {fp.departure_date ? ` → ${fp.departure_date}` : ''}
                      </span>
                      {fp.stay_duration_days > 1 && (
                        <span className="ml-1 text-[10px] font-mono px-1.5 py-0.2 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300 border border-neutral-200/50 dark:border-neutral-700/50">
                          {fp.stay_duration_days} days
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Note / Journal snippet */}
                  {fp.note && (
                    <p className="mt-3 text-xs text-neutral-600 dark:text-neutral-400 italic bg-neutral-50 dark:bg-neutral-950 px-2.5 py-2 rounded border border-neutral-200/60 dark:border-neutral-800 line-clamp-3">
                      "{fp.note}"
                    </p>
                  )}
                </div>

                {/* Card Footer: Category & Spend */}
                <div className="flex items-center justify-between border-t border-neutral-100 dark:border-neutral-850 pt-2.5 mt-3">
                  <span className="text-[10px] uppercase font-mono px-2 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200/50 dark:border-neutral-700/50">
                    {fp.category}
                  </span>
                  {formattedSpend ? (
                    <span className="text-xs font-mono font-medium text-neutral-800 dark:text-neutral-200">
                      {formattedSpend}
                    </span>
                  ) : (
                    <span className="text-[11px] font-mono text-neutral-400">
                      —
                    </span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
