<script lang="ts">
  // A new version (ADR 0033): at the top of Home, the notes told once (the
  // app now looks; a security fix), and at its foot, beside the version,
  // the one line: a newer version, in the footer's muted style, or a
  // security fix, in the amber of warnings. Never a dialog or a sound.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { openAbout } from "./aboutApi";
  import { openDownloadPage, versionNotice, versionTold, type NoticeView } from "./newVersionApi";

  let { place }: { place: "top" | "foot" } = $props();

  let view = $state<NoticeView | null>(null);

  function read() {
    versionNotice()
      .then((seen) => (view = seen))
      .catch(() => {});
  }

  function told(what: "looking" | "security") {
    if (view) view = what === "looking" ? { ...view, tellLooking: false } : { ...view, tellSecurity: false };
    versionTold(what).catch(() => {});
  }

  onMount(() => {
    read();
    const stop = listen("version-notice-changed", read);
    return () => stop.then((unlisten) => unlisten());
  });
</script>

{#if view && place === "top"}
  {#if view.tellSecurity && view.line}
    <div class="note security" role="alert">
      <span>
        {view.line.text}. Get the new version to be safe from it.
        <button class="link" onclick={() => openDownloadPage()}>{view.line.action}</button>
      </span>
      <button onclick={() => told("security")}>OK</button>
    </div>
  {:else if view.tellLooking}
    <div class="note" role="status">
      <span>
        OurStick now looks for a new version once a day, so you won't miss a security fix. You can
        turn this off in
        <button class="link" onclick={() => openAbout()}>About</button>.
      </span>
      <button onclick={() => told("looking")}>OK</button>
    </div>
  {/if}
{/if}

{#if view?.line && place === "foot"}
  <span class="line" class:security={view.line.kind !== "newer" && view.line.kind !== "old"}>
    {view.line.text} ·
    <button class="link" onclick={() => openDownloadPage()}>{view.line.action}</button>
  </span>
{/if}

<style>
  .note {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--accent) 8%, var(--card));
  }
  .note.security {
    border-color: var(--warning);
    background: color-mix(in srgb, var(--warning) 10%, var(--card));
  }
  .line {
    flex: 1 1 auto;
    text-align: center;
    font-size: 0.8em;
    color: var(--muted);
  }
  .line.security {
    color: var(--warning);
    font-weight: 600;
  }
  .link {
    padding: 0;
    border: none;
    background: none;
    min-height: 0;
    font: inherit;
    color: var(--accent);
    cursor: pointer;
  }
  .line.security .link {
    color: inherit;
    text-decoration: underline;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
