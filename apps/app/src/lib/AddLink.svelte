<script lang="ts">
  import { addReceived, chooseFolder, previewLink, type PreviewView } from "./api";
  import { size } from "./format";

  let {
    onAdded,
    onCancel,
  }: {
    onAdded: (stickId: string) => void;
    onCancel: () => void;
  } = $props();

  let text = $state("");
  let parent = $state<string | null>(null);
  let preview = $state<PreviewView | null>(null);
  let looking = $state(false);
  let adding = $state(false);
  let failure = $state<string | null>(null);

  async function look(event?: SubmitEvent) {
    event?.preventDefault();
    looking = true;
    failure = null;
    try {
      preview = await previewLink(text, parent);
    } catch (error) {
      preview = null;
      failure = String(error);
    } finally {
      looking = false;
    }
  }

  async function choose() {
    const picked = await chooseFolder().catch((error) => {
      failure = String(error);
      return null;
    });
    if (picked) {
      parent = picked;
      await look();
    }
  }

  async function getIt() {
    if (!preview) return;
    adding = true;
    failure = null;
    try {
      onAdded(await addReceived(text, preview.folder, preview.name));
    } catch (error) {
      failure = String(error);
    } finally {
      adding = false;
    }
  }

  let fits = $derived(preview !== null && (preview.partial || preview.to_write <= preview.left));
</script>

<section class="add">
  <h1>Add a sharing link</h1>
  <form onsubmit={look}>
    <label>
      Paste the sharing link someone sent you
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={text} autofocus disabled={looking || adding} spellcheck="false" autocomplete="off" />
    </label>
    <div class="row">
      <button type="button" onclick={onCancel} disabled={adding}>Cancel</button>
      <button class="primary" type="submit" disabled={looking || adding || !text.trim()}>Look at it</button>
    </div>
  </form>
  {#if looking}<p class="muted">Looking for the sender… please wait.</p>{/if}
  {#if failure}<p class="error">{failure}</p>{/if}

  {#if preview && !looking}
    <h2><bdi>{preview.name}</bdi></h2>
    <ul class="entries">
      {#each preview.entries as entry (entry.name)}
        <li>
          <bdi class="name">{entry.name}{entry.is_dir ? "/" : ""}</bdi>
          <span class="muted">{size(entry.bytes)}</span>
        </li>
      {/each}
      {#if preview.more > 0}
        <li class="muted">and {preview.more} more</li>
      {/if}
    </ul>
    <p class="row spread">
      <span class="muted path" title={preview.folder}>Save to {preview.folder}</span>
      <button onclick={choose} disabled={adding}>Choose…</button>
    </p>
    {#if preview.earlier}
      <p class="muted">You received it here before with the command line: it goes on in the same folder.</p>
    {/if}
    {#if preview.partial}
      <p class="muted">The sender is still counting its files: more is coming.</p>
    {:else if fits}
      <p class="muted">Needs up to {size(preview.to_write)} · fits</p>
    {:else}
      <p class="error">
        Needs up to {size(preview.to_write)}, and only {size(preview.left)} are left on this computer there. Choose
        another place, or free some room.
      </p>
    {/if}
    {#if adding}<p class="muted">Adding… please wait.</p>{/if}
    <div class="row end">
      <button class="primary" onclick={getIt} disabled={adding || !fits}>Get it</button>
    </div>
  {/if}
</section>

<style>
  .add {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
  }
  h1,
  h2 {
    margin: 0;
  }
  h2 {
    font-size: 1.1em;
  }
  form,
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row {
    justify-content: flex-end;
  }
  .spread {
    justify-content: space-between;
  }
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
    min-height: 0;
    flex: 1 1 auto;
  }
  .entries li {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--button-radius);
  }
  .name,
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    unicode-bidi: isolate;
  }
  p {
    margin: 0;
  }
</style>
