<script lang="ts">
  // Receive's link this computer knows already (ADR 0028 §4): one it sends,
  // one it gets, or one it waits for; with `Show it`.
  import { thisDevice } from "./format";

  let {
    known,
    onAnother,
    onShow,
  }: {
    known: { known: "own" | "received" | "waiting"; id: string; name: string };
    onAnother: () => void;
    onShow: (kind: "sending" | "received", stickId: string) => void;
  } = $props();
</script>

{#if known.known === "own"}
  <h1>This is the link to “<bdi>{known.name}</bdi>”, which you send yourself.</h1>
{:else if known.known === "received"}
  <h1>You already get “<bdi>{known.name}</bdi>” on {thisDevice()}.</h1>
{:else}
  <h1>You already wait for this link.</h1>
{/if}
<div class="actions">
  <button onclick={onAnother}>Paste another link</button>
  <button class="primary" onclick={() => onShow(known.known === "own" ? "sending" : "received", known.id)}
    >Show it</button
  >
</div>
