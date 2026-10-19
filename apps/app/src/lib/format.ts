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

/** "since 14:30" (today), else "since 3 Oct". */
export function since(ms: number): string {
  const when = new Date(ms);
  const today = new Date().toDateString() === when.toDateString();
  return today
    ? `since ${when.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
    : `since ${when.toLocaleDateString([], { day: "numeric", month: "short" })}`;
}

/** What a received stick is doing, in a few words for the list. */
export function receivedShort(entry: { state: string; done: number; total: number }): string {
  switch (entry.state) {
    case "downloading":
      return entry.total > 0 ? `↓ Downloading ${Math.floor((entry.done / entry.total) * 100)}%` : "↓ Downloading…";
    case "getting_more":
      return "✓ All here so far";
    case "up_to_date":
      return "✓ All here. New files come by themselves.";
    case "waiting":
      return "! Waiting";
    case "paused":
      return "⏸ Paused";
    case "stopped":
      return "✕ Stopped: needs you";
    case "waiting_sender":
      return "… Waiting for the sender";
    case "wait_ended":
      return "! The sender did not answer";
    default:
      // Not started yet (the app is still starting).
      return "Getting ready…";
  }
}

/** "Downloads › Holiday photos": the last two parts of a folder's path. */
export function lastParts(path: string): string {
  return path
    .split(/[\\/]/)
    .filter((part) => part)
    .slice(-2)
    .join(" › ");
}

/** "2 folders and 1 file": what a top level holds. */
export function holdsWords(entries: { is_dir: boolean }[]): string {
  const folders = entries.filter((entry) => entry.is_dir).length;
  const plain = entries.length - folders;
  const parts = [];
  if (folders > 0) parts.push(folders === 1 ? "1 folder" : `${folders} folders`);
  if (plain > 0 || folders === 0) parts.push(files(plain));
  return parts.join(" and ");
}
