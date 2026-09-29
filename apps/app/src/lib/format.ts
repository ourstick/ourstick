// Sizes as the mockup shows them: "420 MB", "10 GB", "2.1 MB/s".

export function size(bytes: number): string {
  if (bytes >= 1e9) {
    const gb = bytes / 1e9;
    return `${Number.isInteger(gb) ? gb : gb.toFixed(1)} GB`;
  }
  if (bytes >= 1e6 || bytes === 0) {
    return `${Math.round(bytes / 1e6)} MB`;
  }
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

export function rate(bytesPerSecond: number): string {
  if (bytesPerSecond >= 1e6) {
    return `${(bytesPerSecond / 1e6).toFixed(1)} MB/s`;
  }
  return `${Math.round(bytesPerSecond / 1e3)} KB/s`;
}

export function files(count: number): string {
  return count === 1 ? "1 file" : `${count} files`;
}
