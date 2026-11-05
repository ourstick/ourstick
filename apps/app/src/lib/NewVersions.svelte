<script lang="ts">
  // About's "Look for new versions" (ADR 0033 rule 10): on by default, with
  // what it sends, when it last looked, and a warning when off.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { setLookForVersions, versionNotice, type NoticeView } from "./newVersionApi";

  let view = $state<NoticeView | null>(null);
  let failure = $state<string | null>(null);

  const DAY = 24 * 60 * 60 * 1000;

  function read() {
    versionNotice()
      .then((seen) => (view = seen))
      .catch(() => {});
  }

  async function change(on: boolean) {
    failure = null;
    try {
      await setLookForVersions(on);
    } catch (error) {
      failure = String(error);
    }
    read();
  }

  /** "today", "yesterday", "3 days ago". */
  function since(ms: number): string {
    const days = Math.floor((Date.now() - ms) / DAY);
    if (days <= 0) return "today";
    return days === 1 ? "yesterday" : `${days} days ago`;
  }

  const why: Record<NonNullable<NoticeView["failure"]>, string> = {
    "no-connection": "no connection to ourstick.com",
    refused: "ourstick.com did not give the file",
    "not-the-file": "what came was not OurStick's signed notice",
    old: "ourstick.com gave an older notice than one already seen",
  };

  onMount(() => {
    read();
    const stop = listen("version-notice-changed", read);
    return () => stop.then((unlisten) => unlisten());
  });
</script>

{#if view?.setting}
  <div class="versions">
    <label class="row">
      <input type="checkbox" checked={view.looking} onchange={(event) => change(event.currentTarget.checked)} />
      Look for new versions
    </label>
    <p class="muted small">
      Once a day OurStick asks ourstick.com which version is newest. It sends nothing about you or your files.
    </p>
    {#if !view.looking}
      <p class="warning small">Off, you will not hear of a security fix. Check ourstick.com yourself now and then.</p>
    {:else if view.failingSinceMs !== null && view.failure}
      <p class="muted small">
        Last looked: {view.lastGoodMs !== null ? since(view.lastGoodMs) : "never"}. It could not look now:
        {why[view.failure]}.
      </p>
    {:else if view.lastGoodMs !== null}
      <p class="muted small">Last looked: {since(view.lastGoodMs)}</p>
    {/if}
    {#if failure}<p class="error small">{failure}</p>{/if}
  </div>
{/if}

<style>
  .versions {
    align-self: stretch;
    text-align: left;
    margin-bottom: 6px;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .row input {
    margin-top: 3px;
  }
  .small {
    font-size: 0.85em;
    margin: 2px 0 0 26px;
  }
  .warning {
    color: var(--warning);
  }
</style>
