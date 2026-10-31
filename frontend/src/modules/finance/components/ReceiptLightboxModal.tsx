import React, { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Download, RotateCw, Trash2, X, ZoomIn, ZoomOut } from 'lucide-react';
import { fetchApi } from '../../../core/apiClient';
import { ConfirmModal } from '../../../components/ConfirmModal';
import type { components } from '../../../types/api';

type ReceiptUrl = components['schemas']['ReceiptUrlResponse'];

interface ReceiptLightboxModalProps {
  transactionId: number;
  title: string;
  onClose: () => void;
}

const ZOOM_STEPS = [0.5, 0.75, 1, 1.5, 2, 3];

export const ReceiptLightboxModal: React.FC<ReceiptLightboxModalProps> = ({
  transactionId,
  title,
  onClose,
}) => {
  const queryClient = useQueryClient();
  const [zoomIndex, setZoomIndex] = useState(2);
  const [rotation, setRotation] = useState(0);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [removing, setRemoving] = useState(false);

  const { data, isLoading, error } = useQuery({
    queryKey: ['receipt-url', transactionId],
    queryFn: () => fetchApi<ReceiptUrl>(`/finance/transactions/${transactionId}/receipt-url`),
    // A link lives five minutes; never reuse a stale one from the cache.
    staleTime: 0,
    gcTime: 0,
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !confirmRemove) onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, confirmRemove]);

  const isPdf = data?.url.split('?')[0].toLowerCase().endsWith('.pdf') ?? false;

  const remove = async () => {
    setRemoving(true);
    try {
      await fetchApi(`/finance/transactions/${transactionId}/receipt`, { method: 'DELETE' });
      await queryClient.invalidateQueries({ queryKey: ['transactions'] });
      onClose();
    } finally {
      setRemoving(false);
      setConfirmRemove(false);
    }
  };

  const iconButton =
    'p-1.5 rounded text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed';

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`Receipt for ${title}`}
      data-testid="receipt-lightbox"
      className="fixed inset-0 z-50 flex items-center justify-center bg-neutral-950/70 p-4"
      onClick={onClose}
    >
      <div
        className="flex flex-col w-full max-w-3xl max-h-[90vh] rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-2 px-4 py-2 border-b border-neutral-200 dark:border-neutral-800">
          <p className="text-sm font-medium truncate text-neutral-900 dark:text-neutral-100">
            {title}
          </p>
          <div className="flex items-center gap-1 shrink-0">
            <button
              type="button"
              aria-label="Zoom out"
              disabled={isPdf || zoomIndex === 0}
              onClick={() => setZoomIndex((i) => Math.max(0, i - 1))}
              className={iconButton}
            >
              <ZoomOut className="w-4 h-4" />
            </button>
            <button
              type="button"
              aria-label="Zoom in"
              disabled={isPdf || zoomIndex === ZOOM_STEPS.length - 1}
              onClick={() => setZoomIndex((i) => Math.min(ZOOM_STEPS.length - 1, i + 1))}
              className={iconButton}
            >
              <ZoomIn className="w-4 h-4" />
            </button>
            <button
              type="button"
              aria-label="Rotate 90 degrees"
              disabled={isPdf}
              onClick={() => setRotation((r) => (r + 90) % 360)}
              className={iconButton}
            >
              <RotateCw className="w-4 h-4" />
            </button>
            {data && (
              <a href={data.url} download aria-label="Download receipt" className={iconButton}>
                <Download className="w-4 h-4" />
              </a>
            )}
            <button
              type="button"
              aria-label="Remove receipt"
              onClick={() => setConfirmRemove(true)}
              className={`${iconButton} hover:!text-rose-600`}
            >
              <Trash2 className="w-4 h-4" />
            </button>
            <button type="button" aria-label="Close" onClick={onClose} className={iconButton}>
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>

        <div className="flex-1 overflow-auto p-4 flex items-center justify-center min-h-[40vh]">
          {isLoading && <p className="text-xs font-mono text-neutral-500">Loading receipt...</p>}
          {error && (
            <p className="text-xs font-mono text-rose-600 dark:text-rose-400">
              Could not load this receipt.
            </p>
          )}
          {data &&
            (isPdf ? (
              <iframe src={data.url} title="Receipt PDF" className="w-full h-[70vh]" />
            ) : (
              <img
                src={data.url}
                alt={`Receipt for ${title}`}
                style={{
                  transform: `rotate(${rotation}deg) scale(${ZOOM_STEPS[zoomIndex]})`,
                  transformOrigin: 'center',
                }}
                className="max-w-full max-h-[70vh] object-contain transition-transform duration-150"
              />
            ))}
        </div>
      </div>

      <ConfirmModal
        isOpen={confirmRemove}
        onClose={() => setConfirmRemove(false)}
        onConfirm={remove}
        isLoading={removing}
        title="Remove receipt"
        message="The receipt is unlinked from this entry. The entry itself stays."
        confirmLabel="Remove"
      />
    </div>
  );
};
