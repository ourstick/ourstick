<script lang="ts">
  // A drop that does not fit in what is left (ADR 0008): nothing was copied;
  // "Share it as it is" links it instead, shared from where it is.
  import { linkPaths } from "./api";
  import { size } from "./format";

  let {
    paths,
    needed,
    left,
    onClose,
  }: {
    paths: string[];
    needed: number;
    left: number;
    /** Done: linked (`null`), cancelled (`null`), or why it failed. */
    onClose: (failure: string | null) => void;
  } = $props();

  // Checking and reading the folder can take a moment on a slow disk.
  let linking = $state(false);

  async function shareAsItIs() {
    if (linking) return;
    linking = true;
    try {
      await linkPaths(paths);
      onClose(null);
    } catch (error) {
      onClose(String(error));
    } finally {
      linking = false;
    }
  }
</script>

<div class="notice" role="alert">
  <p>
    Does not fit: {paths.length === 1 ? "it needs" : "they need"} {size(needed)}, and
    {size(left)} {left === 1 ? "is" : "are"} left on this computer.
  </p>
  <p class="muted">
    Share it as it is: shared from where it is, not copied, taking no room on the stick. If you
    move or delete it there, it is gone from the stick too.
  </p>
  <span class="inline-actions">
    {#if linking}<span class="muted">Adding… please wait.</span>{/if}
    <button onclick={() => onClose(null)} disabled={linking}>Cancel</button>
    <button class="primary" onclick={shareAsItIs} disabled={linking}>Share it as it is</button>
  </span>
</div>

<style>
  .notice {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px 12px;
  }
  .notice p {
    margin: 0 0 6px;
  }
  .inline-actions {
    display: flex;
    gap: 6px;
  }
</style>
