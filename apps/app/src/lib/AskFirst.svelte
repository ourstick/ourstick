<script lang="ts">
  // Asked before a main folder of the home or a whole disk is sent
  // (gui-spec, Send, "Asking first"). The safe choice is the blue button.
  import { onMount } from "svelte";
  import { measure, type Measured, type Verdict } from "./sendApi";
  import { files, size } from "./format";

  let {
    verdict,
    onAnswer,
  }: {
    verdict: Verdict;
    /** True: send all of it. False: choose something smaller. */
    onAnswer: (all: boolean) => void;
  } = $props();

  let counted = $state<Measured | null>(null);
  let soFar = $state<Measured | null>(null);

  onMount(() => {
    measure(verdict.path, (measured) => (soFar = measured))
      .then((measured) => (counted = measured))
      .catch(() => {});
  });

  let howMuch = $derived(
    counted
      ? `${files(counted.files)}, ${size(counted.bytes)}`
      : `counting… ${(soFar?.files ?? 0).toLocaleString()} files so far`,
  );
</script>

<div class="dialog-back">
  <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="ask-first-title">
    <h2 id="ask-first-title">{verdict.question}</h2>
    {#if verdict.ask === "main"}
      <p>
        This is one of your main folders: {howMuch}. Folders like this often hold private papers, like bank letters, tax
        forms or passwords.
      </p>
      <p>
        <b>Whoever gets the link sees all of it, and every file you put in “{verdict.name}” later.</b>
      </p>
    {:else}
      <p>
        That is {counted ? size(counted.bytes) : howMuch}. It can take days, and this computer must stay on the whole time.
      </p>
      <p>Usually you want one folder on it, not all of it.</p>
    {/if}
    <div class="actions">
      <button class="danger-outline" onclick={() => onAnswer(true)}>Send all of it</button>
      <!-- svelte-ignore a11y_autofocus -->
      <button class="primary" autofocus onclick={() => onAnswer(false)}>
        {verdict.ask === "main" ? "Choose a smaller folder" : "Choose a folder on it"}
      </button>
    </div>
  </div>
</div>
