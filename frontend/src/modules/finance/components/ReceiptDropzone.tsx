import React, { useRef, useState } from 'react';
import { Camera, Loader2, Check, AlertCircle, X } from 'lucide-react';
import { ReceiptExtraction, ReceiptUploadResponse } from '../types';

interface ReceiptDropzoneProps {
  onReceiptScanned: (extracted: ReceiptExtraction, receiptStorageKey: string) => void;
}

export const ReceiptDropzone: React.FC<ReceiptDropzoneProps> = ({ onReceiptScanned }) => {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [isScanning, setIsScanning] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [scanStatus, setScanStatus] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleFileUpload = async (file: File) => {
    if (!file.type.startsWith('image/') && file.type !== 'application/pdf') {
      setErrorMsg('Please upload a JPG, PNG, WebP or PDF receipt.');
      return;
    }

    setIsScanning(true);
    setErrorMsg(null);
    setScanStatus('Uploading receipt...');

    try {
      const formData = new FormData();
      formData.append('file', file);

      const response = await fetch('/api/v1/finance/receipts/upload', {
        method: 'POST',
        body: formData,
        credentials: 'include',
      });

      if (!response.ok) {
        const errJson = await response.json().catch(() => ({}));
        throw new Error(errJson.error || `Server responded with status ${response.status}`);
      }

      const data: ReceiptUploadResponse = await response.json();
      const extracted = data.extracted || {};

      onReceiptScanned(extracted, data.receipt_storage_key);
      setScanStatus(
        extracted.merchant
          ? `Scanned: ${extracted.merchant} (${extracted.amount ? `${extracted.amount} ` : ''}${extracted.currency || ''})`
          : data.extracted
            ? 'Receipt saved and scanned. Form populated below.'
            : 'Receipt saved. Enter the details below.'
      );
      setTimeout(() => setScanStatus(null), 5000);
    } catch (err: any) {
      setErrorMsg(err.message || 'Failed to scan receipt image.');
      setScanStatus(null);
    } finally {
      setIsScanning(false);
      if (fileInputRef.current) {
        fileInputRef.current.value = '';
      }
    }
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
      handleFileUpload(e.dataTransfer.files[0]);
    }
  };

  return (
    <div className="space-y-2">
      <div
        onDragOver={(e) => {
          e.preventDefault();
          setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={handleDrop}
        onClick={() => fileInputRef.current?.click()}
        data-testid="receipt-dropzone"
        className={`flex items-center justify-between px-3 py-2 rounded-md border border-dashed text-xs cursor-pointer transition-colors duration-150 ${
          dragOver
            ? 'border-neutral-500 bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100'
            : 'border-neutral-300 dark:border-neutral-700 hover:border-neutral-400 dark:hover:border-neutral-600 bg-white dark:bg-neutral-900/60 text-neutral-600 dark:text-neutral-300'
        }`}
      >
        <div className="flex items-center gap-2">
          {isScanning ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin text-neutral-500" />
          ) : (
            <Camera className="w-3.5 h-3.5 text-neutral-500" />
          )}
          <span className="font-mono">
            {isScanning
              ? 'Scanning receipt with OCR...'
              : 'Drop receipt image here or click to scan'}
          </span>
        </div>

        <input
          ref={fileInputRef}
          type="file"
          accept="image/jpeg,image/png,image/webp,application/pdf"
          className="hidden"
          onChange={(e) => {
            if (e.target.files && e.target.files.length > 0) {
              handleFileUpload(e.target.files[0]);
            }
          }}
        />

        <span className="text-[10px] font-mono text-neutral-400">JPG, PNG, WebP, PDF</span>
      </div>

      {scanStatus && (
        <div className="flex items-center justify-between p-2 rounded bg-emerald-50 dark:bg-emerald-950/40 border border-emerald-200 dark:border-emerald-800 text-emerald-800 dark:text-emerald-300 text-xs font-mono">
          <div className="flex items-center gap-1.5">
            <Check className="w-3.5 h-3.5 shrink-0" />
            <span>{scanStatus}</span>
          </div>
          <button
            type="button"
            onClick={() => setScanStatus(null)}
            className="text-emerald-600 hover:text-emerald-900 cursor-pointer"
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      )}

      {errorMsg && (
        <div className="flex items-center justify-between p-2 rounded bg-rose-50 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-800 text-rose-800 dark:text-rose-300 text-xs font-mono">
          <div className="flex items-center gap-1.5">
            <AlertCircle className="w-3.5 h-3.5 shrink-0" />
            <span>{errorMsg}</span>
          </div>
          <button
            type="button"
            onClick={() => setErrorMsg(null)}
            className="text-rose-600 hover:text-rose-900 cursor-pointer"
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      )}
    </div>
  );
};
