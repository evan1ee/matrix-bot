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
} from 'lucide-react';

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
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4 flex flex-col h-full">
      {/* Top Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3 shrink-0">
        <div className="flex items-center gap-2">
          <Compass className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Visited Places</h3>
        </div>
        <span className="text-xs font-mono text-neutral-500">
          {filtered.length} {filtered.length === 1 ? 'place' : 'places'}
        </span>
      </div>

      {/* Search & Add Action */}
      <div className="flex items-center gap-2 shrink-0">
        <div className="relative flex-1">
          <Search className="w-4 h-4 absolute left-3 top-2.5 text-neutral-400" />
          <input
            type="text"
            placeholder="Search places, cities, memories..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full pl-9 pr-3 py-1.5 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
          />
        </div>

        <button
          onClick={onOpenAddModal}
          className="inline-flex items-center justify-center gap-1.5 px-3 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition shrink-0 cursor-pointer"
        >
          <Plus className="w-3.5 h-3.5" />
          <span>Add Place</span>
        </button>
      </div>

      {/* Category Filter Tabs (Matching Task Status Filter) */}
      <div className="flex items-center gap-2 shrink-0">
        <span className="text-xs font-mono text-neutral-500 dark:text-neutral-400 whitespace-nowrap">CATEGORY</span>
        <div className="flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-0.5 text-xs font-mono overflow-x-auto">
          {CATEGORY_FILTERS.map((cat) => (
            <button
              key={cat.id}
              onClick={() => setActiveCategory(cat.id)}
              className={`px-2.5 py-1 rounded capitalize transition whitespace-nowrap ${
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

      {/* Place Items List View */}
      <div className="flex-1 overflow-y-auto divide-y divide-neutral-200 dark:divide-neutral-850 pr-1">
        {filtered.length === 0 ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded flex flex-col items-center gap-2">
            <span>No footprints found.</span>
            <button
              onClick={onOpenAddModal}
              className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition"
            >
              <Plus className="w-3 h-3" />
              <span>Record First Place</span>
            </button>
          </div>
        ) : (
          filtered.map((fp) => {
            const isSelected = selectedFootprint?.id === fp.id;
            const formattedSpend =
              fp.spend_amount_minor > 0
                ? `${(fp.spend_amount_minor / Math.pow(10, fp.spend_currency_exponent)).toFixed(2)} ${fp.spend_currency}`
                : null;

            return (
              <div
                key={fp.id}
                onClick={() => onSelectFootprint(fp)}
                className={`py-3 px-2 rounded transition cursor-pointer ${
                  isSelected
                    ? 'bg-neutral-100 dark:bg-neutral-900 ring-1 ring-neutral-300 dark:ring-neutral-700'
                    : 'hover:bg-neutral-100/50 dark:hover:bg-neutral-900/50'
                }`}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <h4 className="font-medium text-sm text-neutral-900 dark:text-neutral-100 truncate">
                        {fp.title}
                      </h4>
                      <div className="flex items-center text-amber-500 dark:text-amber-400">
                        {Array.from({ length: fp.rating }).map((_, i) => (
                          <Star key={i} className="w-3 h-3 fill-amber-500 dark:fill-amber-400" />
                        ))}
                      </div>
                    </div>

                    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-neutral-500 font-sans mt-1">
                      {(fp.city || fp.country) && (
                        <div className="flex items-center gap-1">
                          <MapPin className="w-3 h-3 text-neutral-400 shrink-0" />
                          <span>{[fp.city, fp.country].filter(Boolean).join(', ')}</span>
                        </div>
                      )}

                      <div className="flex items-center gap-1 font-mono">
                        <Calendar className="w-3 h-3 text-neutral-400 shrink-0" />
                        <span>
                          {fp.arrival_date}
                          {fp.departure_date ? ` → ${fp.departure_date}` : ''}
                        </span>
                      </div>

                      {fp.stay_duration_days > 1 && (
                        <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-neutral-200/70 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300">
                          {fp.stay_duration_days} days
                        </span>
                      )}
                    </div>

                    {fp.note && (
                      <p className="mt-1.5 text-xs text-neutral-600 dark:text-neutral-400 italic bg-neutral-100 dark:bg-neutral-900 px-2.5 py-1.5 rounded border border-neutral-200/60 dark:border-neutral-800 line-clamp-2">
                        "{fp.note}"
                      </p>
                    )}

                    <div className="flex items-center gap-2 mt-2">
                      <span className="text-[10px] uppercase font-mono px-1.5 py-0.5 rounded bg-neutral-200/50 dark:bg-neutral-800/80 text-neutral-600 dark:text-neutral-400">
                        {fp.category}
                      </span>
                      {formattedSpend && (
                        <span className="text-xs font-mono font-medium text-neutral-700 dark:text-neutral-300">
                          {formattedSpend}
                        </span>
                      )}
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
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};
