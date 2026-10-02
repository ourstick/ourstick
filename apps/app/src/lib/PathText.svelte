<script lang="ts">
  // A path on one line whose own folder's name stays in view: the start
  // gives way first ("/Users/me/Downl…/Holiday 2026"), whatever the width.
  let { path, label = "" }: { path: string; label?: string } = $props();
  let cut = $derived(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")));
  // The separator goes with the name, so it shows as "…/Holiday 2026".
  let start = $derived(cut >= 0 ? path.slice(0, cut) : "");
  let last = $derived(cut >= 0 ? path.slice(cut) : path);
</script>

<span class="path" title={path}>
  {#if label}<span class="label">{label}&nbsp;</span>{/if}<span class="start">{start}</span><bdi class="last"
    >{last}</bdi
  >
</span>

<style>
  .path {
    display: flex;
    min-width: 0;
    white-space: nowrap;
  }
  .label {
    flex-shrink: 0;
  }
  .start {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 1.5em;
  }
  .last {
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 0;
    max-width: 75%;
  }
</style>
