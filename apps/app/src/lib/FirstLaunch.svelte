<script lang="ts">
  import { plugIn } from "./api";

  let { onDone, onReceive }: { onDone: () => void; onReceive: () => void } = $props();
  let name = $state("OurStick");
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function plug(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    failure = null;
    try {
      await plugIn(name);
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
    USB drive. Put files on it, then send its sharing link — the other person sees
    the same stick. It holds as much as your computer has room for.
  </p>
  <form onsubmit={plug}>
    <label>
      Name of your stick
      <input bind:value={name} maxlength="60" required disabled={busy} />
    </label>
    {#if failure}<p class="error">{failure}</p>{/if}
    <div class="actions plug">
      <button class="primary" type="submit" disabled={busy || !name.trim()}>Plug it in</button>
    </div>
  </form>
  <p class="receive">
    Someone sent you a sharing link?
    <button type="button" onclick={onReceive} disabled={busy}>I got a sharing link</button>
  </p>
</main>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .receive {
    margin-top: 20px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    color: var(--muted);
  }
  /* Centred with the text, not pushed to the bottom. */
  .actions.plug {
    margin-top: 16px;
  }
</style>
