<script lang="ts">
  // ADR 0021: once, the keys move from the login keychain, which asks for
  // the password, to the data protection keychain, which never asks.
  import { moveKeys } from "./macApi";

  let { keys, onDone }: { keys: number; onDone: () => void } = $props();
  let busy = $state(false);
  let failure = $state<string | null>(null);

  async function go(later: boolean) {
    busy = true;
    failure = null;
    try {
      await moveKeys(later);
      onDone();
    } catch (error) {
      failure = String(error);
    } finally {
      busy = false;
    }
  }
</script>

<main class="card centred">
  <h1>One last password</h1>
  <p>
    OurStick is moving its keys to a part of the keychain that never asks for your password again. To read
    them one last time, macOS asks for it up to {keys} {keys === 1 ? "time" : "times"}: each time, type it and
    choose Always Allow.
  </p>
  {#if busy}<p class="muted">Moving the keys… answer macOS when it asks.</p>{/if}
  {#if failure}
    <p class="error">{failure}</p>
    <p class="muted">Your keys are where they were, and everything works as before.</p>
  {/if}
  <div class="actions">
    {#if failure}<button onclick={() => go(true)} disabled={busy}>Not now</button>{/if}
    <button class="primary" onclick={() => go(false)} disabled={busy}>{failure ? "Try again" : "Continue"}</button>
  </div>
</main>
