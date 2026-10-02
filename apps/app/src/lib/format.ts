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
    case "starting":
      return "Getting ready…";
    case "downloading":
      return entry.total > 0 ? `Downloading ${Math.floor((entry.done / entry.total) * 100)}%` : "Downloading…";
    case "getting_more":
      return "Getting more…";
    case "up_to_date":
      return "Up to date";
    case "waiting":
      return "Waiting";
    case "paused":
      return "Paused";
    case "stopped":
      return "Stopped";
    default:
      return "";
  }
}
