import React, { useRef, useState } from 'react';
import { Download, Upload, X } from 'lucide-react';
import { useExportNotes, useImportNotes } from './api';
import type { ImportReport } from './types';

const BUTTON =
  'flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-lg border transition-colors duration-150 ' +
  'bg-white dark:bg-neutral-900 text-neutral-700 dark:text-neutral-300 border-neutral-200 dark:border-neutral-800 ' +
  'hover:bg-neutral-50 dark:hover:bg-neutral-800 focus-visible:outline-2 focus-visible:outline-neutral-500 ' +
  'disabled:opacity-50 disabled:cursor-not-allowed';

function summary(report: ImportReport): string {
  const parts = [`${report.created} created`, `${report.updated} updated`];
  if (report.skipped.length > 0) parts.push(`${report.skipped.length} skipped`);
  return parts.join(', ');
}

/** Export all notes as a zip, or import a zip of Markdown files. */
export const NotesTransfer: React.FC = () => {
  const fileInput = useRef<HTMLInputElement>(null);
  const [report, setReport] = useState<ImportReport | null>(null);
  const exportNotes = useExportNotes();
  const importNotes = useImportNotes();

  const error = exportNotes.error ?? importNotes.error;
  const busy = exportNotes.isPending || importNotes.isPending;

  const handleFile = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    // Clearing the input lets the same file be chosen again after a failure.
    event.target.value = '';
    if (!file) return;
    setReport(null);
    exportNotes.reset();
    try {
      setReport(await importNotes.mutateAsync(file));
    } catch {
      // The mutation holds the error; it is shown below.
    }
  };

  const handleExport = () => {
    setReport(null);
    importNotes.reset();
    exportNotes.mutate();
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <button
          type="button"
          data-testid="export-notes-button"
          onClick={handleExport}
          disabled={busy}
          className={BUTTON}
        >
          <Download className="w-3.5 h-3.5" />
          <span>{exportNotes.isPending ? 'Exporting…' : 'Export'}</span>
        </button>
        <button
          type="button"
          data-testid="import-notes-button"
          onClick={() => fileInput.current?.click()}
          disabled={busy}
          className={BUTTON}
        >
          <Upload className="w-3.5 h-3.5" />
          <span>{importNotes.isPending ? 'Importing…' : 'Import'}</span>
        </button>
        <input
          ref={fileInput}
          type="file"
          accept=".zip,application/zip"
          aria-label="Zip file of Markdown notes to import"
          data-testid="import-notes-input"
          className="hidden"
          onChange={handleFile}
        />
      </div>

      {error && (
        <p role="alert" className="text-xs text-red-600 dark:text-red-400">
          {error.message}
        </p>
      )}

      {report && (
        <div
          role="status"
          data-testid="import-report"
          className="rounded-lg border border-neutral-200 dark:border-neutral-800 p-3 text-xs space-y-2"
        >
          <div className="flex items-start justify-between gap-2">
            <p className="font-medium text-neutral-800 dark:text-neutral-200">
              Import finished: {summary(report)}
            </p>
            <button
              type="button"
              aria-label="Dismiss import summary"
              onClick={() => setReport(null)}
              className="text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 transition-colors duration-150 focus-visible:outline-2 focus-visible:outline-neutral-500"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
          {report.skipped.length > 0 && (
            <ul className="space-y-1 text-neutral-600 dark:text-neutral-400">
              {report.skipped.map((item) => (
                <li key={item.file}>
                  <span className="font-mono">{item.file}</span>: {item.reason}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
};
