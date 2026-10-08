<script lang="ts">
  // ADR 0020: a transfer is slow because macOS declines OurStick the local
  // network, and the other computer is on it. Stays while that holds.
  import { openLocalNetworkSettings } from "./macApi";

  let { other }: { other: string } = $props();
  let failure = $state<string | null>(null);

  async function open() {
    failure = null;
    try {
      await openLocalNetworkSettings();
    } catch (error) {
      failure = String(error);
    }
  }
</script>

<div class="note">
  <p class="error">
    This is slow: {other} is on your network, but OurStick is not allowed to reach it.
    <span class="muted">(System Settings → Privacy &amp; Security → Local Network)</span>
  </p>
  <p><button onclick={open}>Open Settings</button></p>
  {#if failure}<p class="error">{failure}</p>{/if}
</div>
