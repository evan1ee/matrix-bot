import React, { useState, useCallback } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import { Footprint, CreateFootprintInput, FootprintsSummaryStats } from './types';
import { FootprintsMap } from './FootprintsMap';
import { FootprintsList } from './FootprintsList';
import { FootprintModal } from './FootprintModal';

export * from './types';

const FootprintsView: React.FC<{
  events?: SystemEvent[];
  isConnected?: boolean;
}> = () => {
  const queryClient = useQueryClient();

  const [selectedFootprint, setSelectedFootprint] = useState<Footprint | null>(null);
  const [editingFootprint, setEditingFootprint] = useState<Footprint | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [pendingPin, setPendingPin] = useState<{ lat: number; lon: number } | null>(null);

  // Queries
  const { data: footprints = [], isLoading } = useQuery({
    queryKey: ['footprints'],
    queryFn: () => fetchApi<Footprint[]>('/footprints'),
  });

  const { data: summaryStats = null } = useQuery({
    queryKey: ['footprints-stats'],
    queryFn: () => fetchApi<FootprintsSummaryStats>('/footprints/stats'),
  });

  // Mutations
  const createMutation = useMutation({
    mutationFn: (data: CreateFootprintInput) =>
      fetchApi<Footprint>('/footprints', {
        method: 'POST',
        body: JSON.stringify(data),
      }),
    onSuccess: (newFp) => {
      queryClient.invalidateQueries({ queryKey: ['footprints'] });
      queryClient.invalidateQueries({ queryKey: ['footprints-stats'] });
      setSelectedFootprint(newFp);
      setPendingPin(null);
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: number; data: CreateFootprintInput }) =>
      fetchApi<Footprint>(`/footprints/${id}`, {
        method: 'PUT',
        body: JSON.stringify(data),
      }),
    onSuccess: (updatedFp) => {
      queryClient.invalidateQueries({ queryKey: ['footprints'] });
      queryClient.invalidateQueries({ queryKey: ['footprints-stats'] });
      setSelectedFootprint(updatedFp);
      setEditingFootprint(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi<{ success: boolean; id: number }>(`/footprints/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: (res) => {
      queryClient.invalidateQueries({ queryKey: ['footprints'] });
      queryClient.invalidateQueries({ queryKey: ['footprints-stats'] });
      if (selectedFootprint?.id === res.id) {
        setSelectedFootprint(null);
      }
    },
  });

  const handleMapClick = useCallback((lat: number, lon: number) => {
    setPendingPin({ lat, lon });
    setEditingFootprint(null);
    setIsModalOpen(true);
  }, []);

  const handleSaveFootprint = async (data: CreateFootprintInput) => {
    if (editingFootprint) {
      await updateMutation.mutateAsync({ id: editingFootprint.id, data });
    } else {
      await createMutation.mutateAsync(data);
    }
  };

  const handleOpenAdd = () => {
    setEditingFootprint(null);
    setPendingPin(null);
    setIsModalOpen(true);
  };

  const handleOpenEdit = (fp: Footprint) => {
    setEditingFootprint(fp);
    setPendingPin(null);
    setIsModalOpen(true);
  };

  // Spend metric from summary across all currencies
  const formattedTotalSpend = summaryStats?.spend_by_currency?.length
    ? summaryStats.spend_by_currency
        .map(
          (s) =>
            `${(s.total_spend_minor / Math.pow(10, s.currency_exponent)).toLocaleString(undefined, {
              minimumFractionDigits: 2,
              maximumFractionDigits: 2,
            })} ${s.currency}`
        )
        .join(' · ')
    : '—';

  return (
    <div className="space-y-6 pb-8">


      {/* Vercel Stat Strip (Design.md vbg-stat-strip) */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800">
        <div className="flex flex-col">
          <span className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider">
            Places Visited
          </span>
          <span className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 font-sans tabular-nums mt-0.5">
            {summaryStats?.total_places ?? footprints.length}
          </span>
        </div>

        <div className="flex flex-col">
          <span className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider">
            Countries
          </span>
          <span className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 font-sans tabular-nums mt-0.5">
            {summaryStats?.total_countries ?? 0}
          </span>
        </div>

        <div className="flex flex-col">
          <span className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider">
            Days Traveled
          </span>
          <span className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 font-sans tabular-nums mt-0.5">
            {summaryStats?.total_days_traveled ?? 0}
          </span>
        </div>

        <div className="flex flex-col min-w-0">
          <span className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider">
            Recorded Spend
          </span>
          {summaryStats?.spend_by_currency?.length ? (
            <div
              className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 mt-0.5"
              title={formattedTotalSpend}
            >
              {summaryStats.spend_by_currency.map((s, idx) => (
                <span
                  key={s.currency}
                  className="text-lg sm:text-xl font-semibold text-neutral-900 dark:text-neutral-100 font-sans tabular-nums whitespace-nowrap inline-flex items-baseline gap-1"
                >
                  {(s.total_spend_minor / Math.pow(10, s.currency_exponent)).toLocaleString(undefined, {
                    minimumFractionDigits: 2,
                    maximumFractionDigits: 2,
                  })}
                  <span className="text-[11px] font-mono font-normal text-neutral-500">
                    {s.currency}
                  </span>
                  {idx < summaryStats.spend_by_currency.length - 1 && (
                    <span className="text-neutral-300 dark:text-neutral-700 font-normal select-none">·</span>
                  )}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-xl font-semibold text-neutral-900 dark:text-neutral-100 font-sans tabular-nums mt-0.5">
              —
            </span>
          )}
        </div>
      </div>

      {/* TOP: Interactive World Map (Full Width) */}
      <div className="w-full">
        <FootprintsMap
          footprints={footprints}
          selectedFootprint={selectedFootprint}
          onSelectFootprint={setSelectedFootprint}
          onMapClick={handleMapClick}
          pendingPin={pendingPin}
        />
      </div>

      {/* DOWN: Card Grid & Filter List (Full Width) */}
      <div className="w-full">
        <FootprintsList
          footprints={footprints}
          summaryStats={summaryStats}
          selectedFootprint={selectedFootprint}
          onSelectFootprint={setSelectedFootprint}
          onEditFootprint={handleOpenEdit}
          onDeleteFootprint={(id) => deleteMutation.mutate(id)}
          onOpenAddModal={handleOpenAdd}
        />
      </div>

      {/* Modal Dialog for Recording / Editing */}
      <FootprintModal
        isOpen={isModalOpen}
        onClose={() => {
          setIsModalOpen(false);
          setPendingPin(null);
          setEditingFootprint(null);
        }}
        onSave={handleSaveFootprint}
        initialData={editingFootprint}
        initialCoordinates={pendingPin}
      />
    </div>
  );
};

export const footprintsModule: FrontendModule = {
  id: 'footprints',
  name: 'Footprints',
  description: 'Track visited places on an interactive star map with memories and spending',
  iconName: 'Compass',
  navTab: {
    label: 'Footprints',
  },
  MainView: FootprintsView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'FootprintCreated' ||
      event.type === 'FootprintUpdated' ||
      event.type === 'FootprintDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['footprints'] });
      queryClient.invalidateQueries({ queryKey: ['footprints-stats'] });
    }
  },
};
