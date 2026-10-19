<script lang="ts" module>
  /** What the "⋯" menu of something you send offers. */
  export type Pick = "rename" | "new-link" | "show-files" | "stop" | "start" | "remove";
</script>

<script lang="ts">
  let {
    sending,
    ownFolder,
    open = $bindable(false),
    onPick,
  }: {
    sending: boolean;
    /** A stick made before ADR 0028: its own folder can be shown. */
    ownFolder: boolean;
    open: boolean;
    onPick: (pick: Pick) => void;
  } = $props();

  function pick(choice: Pick) {
    open = false;
    onPick(choice);
  }
</script>

<div class="menu-box">
  <button class="more" aria-label="More" aria-expanded={open} onclick={() => (open = !open)}>⋯</button>
  {#if open}
    <div class="menu" role="menu">
      <button role="menuitem" onclick={() => pick("rename")}>Rename…</button>
      <button role="menuitem" onclick={() => pick("new-link")}>
        <span>Make a new link…</span><span class="hint">The old link stops working.</span>
      </button>
      {#if ownFolder}
        <button role="menuitem" onclick={() => pick("show-files")}>Show the files on this computer</button>
      {/if}
      {#if sending}
        <button role="menuitem" onclick={() => pick("stop")}>
          <span>Stop sending…</span><span class="hint">You can start again later. The link stays.</span>
        </button>
      {:else}
        <button role="menuitem" onclick={() => pick("start")}>Resume</button>
      {/if}
      <button role="menuitem" class="danger-text" onclick={() => pick("remove")}>
        <span>Remove from OurStick…</span><span class="hint">Your files stay where they are.</span>
      </button>
    </div>
  {/if}
</div>

<style>
  .more {
    padding: 0 12px;
    font-weight: 700;
  }
  .menu button {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    height: auto;
    padding: 8px 12px;
  }
  .hint {
    font-size: 0.85em;
    color: var(--muted);
  }
</style>
