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

/**
 * A long path cut in the middle, so its own folder's name stays in view:
 * "/Users/me/…/Holiday 2026". Either separator (macOS, Linux, Windows).
 */
export function middle(path: string, max = 48): string {
  const chars = [...path];
  if (chars.length <= max) return path;
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const separator = cut >= 0 ? path[cut] : "";
  const last = [...path.slice(cut + 1)];
  // The last name alone is too long: its end stays.
  if (last.length + 2 >= max) return "…" + last.slice(last.length - (max - 1)).join("");
  const start = chars.slice(0, max - last.length - 2).join("");
  return `${start}…${separator}${last.join("")}`;
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
      // Not started yet (the app is still starting).
      return "Getting ready…";
  }
}
