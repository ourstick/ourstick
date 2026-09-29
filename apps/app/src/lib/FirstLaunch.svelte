<script lang="ts">
  import { plugIn, STICK_SIZES_GB } from "./api";

  let { onDone }: { onDone: () => void } = $props();
  let size = $state(10);
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function plug() {
    busy = true;
    failure = null;
    try {
      await plugIn(size);
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
    the same stick.
  </p>
  <label class="row">
    Stick size:
    <select bind:value={size} disabled={busy}>
      {#each STICK_SIZES_GB as gb}
        <option value={gb}>{gb} GB</option>
      {/each}
    </select>
  </label>
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
