<script lang="ts">
  // The window (ADR 0028): one column. Home, then a screen at a time, each
  // with `‹ Back` to Home.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getVersion } from "@tauri-apps/api/app";
  import { openAbout, setStartAtLogin, startAtLogin, startAtLoginTold } from "./aboutApi";
  import { linkOpened, newLinkOnClipboard } from "./macApi";
  import {
    appProblem,
    dismissAppProblem,
    onPhone,
    received,
    sticks,
    type About,
    type Activity,
    type ReceivedEntry,
    type ShareProblem,
    type StickEntry,
  } from "./api";
  import Home from "./Home.svelte";
  import LinkReady from "./LinkReady.svelte";
  import ReceiveView from "./ReceiveView.svelte";
  import ReceivedView from "./ReceivedView.svelte";
  import SendView from "./SendView.svelte";
  import StickView from "./StickView.svelte";
  import WaitingView from "./WaitingView.svelte";
  import type { Outcome } from "./StickAsk.svelte";

  let { startAdding = false }: { startAdding?: boolean } = $props();

  let sending = $state<StickEntry[]>([]);
  let receivedEntries = $state<ReceivedEntry[]>([]);
  /** The screen shown. */
  type Shown =
    | { kind: "home" }
    | { kind: "send" }
    | { kind: "link"; id: string }
    | { kind: "sending"; id: string }
    | { kind: "received"; id: string }
    | { kind: "receive" };
  // svelte-ignore state_referenced_locally
  let shown = $state<Shown>(startAdding ? { kind: "receive" } : { kind: "home" });
  let receivedShown = $derived.by(() => {
    const now = shown;
    return now.kind === "received" ? (receivedEntries.find((entry) => entry.id === now.id) ?? null) : null;
  });
  let failure = $state<string | null>(null);
  /** Shown at the bottom, so a report can say which one. */
  let version = $state("");
  /** Said once: the app starts at login, with its check (ADR 0024). */
  let loginNotice = $state(false);
  let loginOn = $state(true);
  /** A problem of the whole app, not of one thing. */
  let problem = $state<ShareProblem | null>(null);
  /** Answers to an older look are dropped: they arrive in any order. */
  let looks = 0;
  // A phone's back gesture (ADR 0027): a screen other than Home is one
  // step in the web view's history, which the system's back goes back on.
  const phone = onPhone();
  $effect(() => {
    if (!phone) return;
    const away = shown.kind !== "home";
    const marked = history.state?.away === true;
    if (away && !marked) history.pushState({ away: true }, "");
    else if (!away && marked) history.back();
  });

  function backToHome() {
    if (history.state?.away !== true) shown = { kind: "home" };
  }

  /** Links opened from another app: each starts Receive afresh. */
  let opened = $state(0);
  /** Receive is past "Paste the link": one opened from outside (a desktop's)
   * waits for the next Receive instead of ending it. */
  let receiveBusy = false;

  function linkArrived() {
    if (!phone && shown.kind === "receive" && receiveBusy) return;
    opened++;
    shown = { kind: "receive" };
  }

  // Back on Home after the window lost focus (ADR 0030 §4): a link not
  // seen before on the clipboard opens Receive, which asks before it looks.
  // Not the first focus after the start; the core says no on a Mac.
  let away = false;
  function blurred() {
    away = true;
  }
  function focused() {
    if (!away || phone || shown.kind !== "home") return;
    away = false;
    void newLinkOnClipboard()
      .then((found) => found && shown.kind === "home" && linkArrived())
      .catch(() => {});
  }

  async function refresh() {
    const look = ++looks;
    try {
      const [loaded, coming, appWide] = await Promise.all([sticks(), received(), appProblem()]);
      if (look !== looks) return;
      sending = loaded;
      receivedEntries = coming;
      problem = appWide;
      failure = null;
      // A screen of something no longer here goes back home.
      const now = shown;
      const gone =
        ((now.kind === "sending" || now.kind === "link") && !sending.some((entry) => entry.id === now.id)) ||
        (now.kind === "received" && !receivedEntries.some((entry) => entry.id === now.id));
      if (gone) shown = { kind: "home" };
    } catch (error) {
      if (look === looks) failure = String(error);
    }
  }

  function home() {
    shown = { kind: "home" };
    refresh();
  }

  async function dismiss() {
    problem = null;
    await dismissAppProblem().catch((error) => (failure = String(error)));
  }

  function followed(outcome: Outcome) {
    // A new link is a new id: the view follows it.
    if (outcome?.kind === "new-link") shown = { kind: "sending", id: outcome.id };
    refresh();
  }

  getVersion().then((v) => (version = v)).catch(() => {});
  startAtLogin()
    .then((login) => {
      loginOn = login.on;
      loginNotice = login.on && !login.told;
    })
    .catch(() => {});

  async function loginChecked() {
    await setStartAtLogin(loginOn).catch((error) => (failure = String(error)));
  }

  function loginNoticeRead() {
    loginNotice = false;
    startAtLoginTold().catch(() => {});
  }

  onMount(() => {
    refresh();
    const stops = [
      listen("sticks-changed", refresh),
      listen("sharing-changed", refresh),
      listen("mount-changed", refresh),
      listen("problems-changed", refresh),
      listen("received-changed", refresh),
      listen("link-opened", linkArrived),
      // The list shows who gets it, not how fast: only a new count matters.
      listen<About<Activity>>("activity", (event) => {
        const entry = sending.find((entry) => entry.id === event.payload.stick_id);
        if (entry && entry.readers !== event.payload.readers) refresh();
      }),
      // The tray: a thing's view, Send… or Receive….
      listen<{ kind: "sending" | "received" | "send" | "receive"; id: string | null }>("show", (event) => {
        const { kind, id } = event.payload;
        if ((kind === "sending" || kind === "received") && id) shown = { kind, id };
        else if (kind === "send" || kind === "receive") shown = { kind };
        refresh();
      }),
    ];
    window.addEventListener("popstate", backToHome);
    window.addEventListener("blur", blurred);
    window.addEventListener("focus", focused);
    // A link opened before the window listened (the app started with it).
    void Promise.all(stops)
      .then(linkOpened)
      .then((waiting) => waiting && opened === 0 && linkArrived())
      .catch(() => {});
    return () => {
      window.removeEventListener("popstate", backToHome);
      window.removeEventListener("blur", blurred);
      window.removeEventListener("focus", focused);
      stops.forEach((stop) => stop.then((unlisten) => unlisten()));
    };
  });
</script>

<main class="card window">
  {#if loginNotice && shown.kind === "home"}
    <div class="login-notice" role="status">
      <span class="text">
        OurStick opens by itself when you turn on the computer, so what you send keeps going.
        <label>
          <input type="checkbox" bind:checked={loginOn} onchange={loginChecked} />
          Open OurStick when the computer starts
        </label>
      </span>
      <button onclick={loginNoticeRead}>OK</button>
    </div>
  {/if}
  {#if problem?.kind === "failed"}
    <p class="app-problem error" role="alert">
      <span>{problem.message}</span>
      <button class="close" aria-label="Close" onclick={dismiss}>×</button>
    </p>
  {/if}

  <div class="screen">
    {#if shown.kind === "home"}
      <Home
        {sending}
        received={receivedEntries}
        onSend={() => (shown = { kind: "send" })}
        onReceive={() => (shown = { kind: "receive" })}
        onOpenSending={(id) => (shown = { kind: "sending", id })}
        onOpenReceived={(id) => (shown = { kind: "received", id })}
      />
    {:else if shown.kind === "send"}
      <SendView onBack={home} onSent={(id) => ((shown = { kind: "link", id }), refresh())} />
    {:else if shown.kind === "link"}
      {@const id = shown.id}
      {#key id}
        <LinkReady stickId={id} onBack={home} onSeeWho={() => (shown = { kind: "sending", id })} />
      {/key}
    {:else}
      <button class="back" onclick={home}>‹ Back</button>
      {#if shown.kind === "sending"}
        {#key shown.id}
          <StickView stickId={shown.id} onOutcome={followed} />
        {/key}
      {:else if shown.kind === "received" && receivedShown}
        {#key shown.id}
          {#if receivedShown.waiting}
            <WaitingView entry={receivedShown} onForgotten={home} />
          {:else}
            <ReceivedView entry={receivedShown} onForgotten={home} />
          {/if}
        {/key}
      {:else if shown.kind === "receive"}
        {#key opened}
          <ReceiveView
            onBusy={(busy) => (receiveBusy = busy)}
            onAdded={(id) => ((shown = { kind: "received", id }), refresh())}
            onShow={(kind, id) => ((shown = { kind, id }), refresh())}
          />
        {/key}
      {/if}
    {/if}
    {#if failure}<p class="error">{failure}</p>{/if}
  </div>

  {#if version && shown.kind === "home"}
    <button class="version muted" title="About OurStick" onclick={() => openAbout()}>OurStick {version}</button>
  {/if}
</main>

<style>
  .window {
    gap: 12px;
  }
  .screen {
    display: flex;
    flex-direction: column;
    gap: 12px;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }
  .back {
    align-self: flex-start;
    border: none;
    background: none;
    padding: 0;
    min-height: 0;
    color: var(--accent);
    cursor: pointer;
  }
  .login-notice {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--accent) 8%, var(--card));
  }
  .login-notice .text {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .login-notice label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .version {
    align-self: flex-start;
    padding: 0;
    border: none;
    background: none;
    min-height: 0;
    font-size: 0.8em;
    cursor: pointer;
  }
  .version:hover {
    text-decoration: underline;
  }
  .app-problem {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    white-space: pre-line;
  }
  .close {
    padding: 0 8px;
  }
</style>
