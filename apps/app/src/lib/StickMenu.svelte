<script lang="ts" module>
  /** What a stick's "⋯" menu offers. */
  export type Pick = "rename" | "new-link" | "stop" | "start" | "remove";
</script>

<script lang="ts">
  let { sharing, onPick }: { sharing: boolean; onPick: (pick: Pick) => void } = $props();
  let open = $state(false);

  function pick(choice: Pick) {
    open = false;
    onPick(choice);
  }
</script>

<div class="menu-box">
  <button class="more" aria-label="More for this stick" aria-expanded={open} onclick={() => (open = !open)}>⋯</button>
  {#if open}
    <div class="menu" role="menu">
      <button role="menuitem" onclick={() => pick("rename")}>Rename…</button>
      {#if sharing}
        <button role="menuitem" onclick={() => pick("stop")}>Stop sharing…</button>
      {:else}
        <button role="menuitem" onclick={() => pick("start")}>Start sharing</button>
      {/if}
      <button role="menuitem" onclick={() => pick("new-link")}>Give it a new link…</button>
      <button role="menuitem" class="danger-text" onclick={() => pick("remove")}>Remove this stick…</button>
    </div>
  {/if}
</div>

<style>
  .more {
    padding: 0 10px;
    font-weight: 700;
  }
</style>
