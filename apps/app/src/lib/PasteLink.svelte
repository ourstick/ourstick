<script lang="ts">
  // Receive's "Paste the link you got" (ADR 0028 §4): how to copy it, and
  // the field. The link is never shown elsewhere, logged or put in a message.
  let {
    text = $bindable(),
    onOpen,
    onPaste,
  }: {
    text: string;
    onOpen: (event: SubmitEvent) => void;
    /** A phone: the clipboard is read only when this is tapped. */
    onPaste?: () => void;
  } = $props();
</script>

<h1>Paste the link you got</h1>
<ol class="steps">
  <li>In the email or message you got, copy the whole link. It starts with <b>ourstick.com/s/</b></li>
  {#if onPaste}
    <li>Come back here and tap <b>Paste the link</b>.</li>
  {:else}
    <li>Come back here: OurStick finds it by itself. Or paste it below.</li>
  {/if}
</ol>
{#if onPaste}<button class="primary paste" onclick={onPaste}>Paste the link</button>{/if}
<form onsubmit={onOpen}>
  <label for="the-link">The link</label>
  <div class="row">
    <!-- svelte-ignore a11y_autofocus -->
    <input
      id="the-link"
      bind:value={text}
      placeholder="https://ourstick.com/s/#…"
      autofocus={!onPaste}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="primary" type="submit" disabled={!text.trim()}>Open</button>
  </div>
</form>
<p class="muted">Clicked the link and a web page opened? Press “Copy the link” on that page, then come back here.</p>

<style>
  h1 {
    margin: 0;
    overflow-wrap: anywhere;
  }
  p {
    margin: 0;
  }
  .steps {
    margin: 0;
    padding-left: 22px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .paste {
    align-self: flex-start;
  }
  form input {
    flex: 1 1 auto;
    min-width: 0;
  }
</style>
