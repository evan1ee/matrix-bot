/** Hands `blob` to the browser as a file download named `filename`. */
export function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
  // Revoking in the same tick can cancel the download in some browsers.
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** The `filename` of a `Content-Disposition` header, if it names one. */
export function filenameFromDisposition(header: string | null): string | null {
  const match = header?.match(/filename="([^"\r\n]+)"/i);
  return match ? match[1] : null;
}
