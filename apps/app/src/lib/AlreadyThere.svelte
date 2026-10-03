<script lang="ts">
  // A drop whose names are already on the stick: asked before anything is
  // copied, as a file manager asks.
  let {
    names,
    onAnswer,
  }: {
    /** The names already there, as the user's own files name them. */
    names: string[];
    /** "replace", "keep_both", or null to cancel. */
    onAnswer: (choice: "replace" | "keep_both" | null) => void;
  } = $props();
</script>

<div class="ask" role="alertdialog" aria-label="Already on the stick" tabindex="-1">
  <p>
    {#if names.length === 1}
      “<bdi>{names[0]}</bdi>” is already on the stick.
    {:else}
      “<bdi>{names[0]}</bdi>” and {names.length - 1}
      {names.length === 2 ? "more is" : "more are"} already on the stick.
    {/if}
  </p>
  <p class="muted">Replace changes only the stick's copy, never the original.</p>
  <div class="actions">
    <button onclick={() => onAnswer(null)}>Cancel</button>
    <button onclick={() => onAnswer("keep_both")}>Keep both</button>
    <button class="primary" onclick={() => onAnswer("replace")}>Replace</button>
  </div>
</div>

<style>
  .ask {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  p {
    margin: 0;
  }
</style>
