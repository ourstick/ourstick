<script lang="ts">
  import { plugIn } from "./api";

  let { onDone }: { onDone: () => void } = $props();
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function plug() {
    busy = true;
    failure = null;
    try {
      await plugIn();
      onDone();
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }
</script>

<main class="card centred">
  <h1>Welcome to OurStick</h1>
  <p>
    OurStick is a virtual USB stick. It appears in your file manager like any
    USB drive. Put files on it, then share the link — the other person sees
    the same stick. It holds as much as your computer has room for.
  </p>
  {#if failure}<p class="error">{failure}</p>{/if}
  <div class="actions plug">
    <button class="primary" onclick={plug} disabled={busy}>Plug it in</button>
  </div>
</main>

<style>
  /* Centred with the text, not pushed to the bottom. */
  .card > .actions.plug {
    margin-top: 16px;
  }
</style>
