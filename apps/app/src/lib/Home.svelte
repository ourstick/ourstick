<script lang="ts">
  // Home (gui-spec, Home): Send and Receive, then what you send and what
  // you received, one row each with one sentence.
  import { onPhone, type ReceivedEntry, type StickEntry } from "./api";
  import { receivedShort } from "./format";

  let {
    sending,
    received,
    onSend,
    onReceive,
    onOpenSending,
    onOpenReceived,
  }: {
    sending: StickEntry[];
    received: ReceivedEntry[];
    onSend: () => void;
    onReceive: () => void;
    onOpenSending: (id: string) => void;
    onOpenReceived: (id: string) => void;
  } = $props();

  /** One sentence, with its symbol, for a thing you send. */
  function sendingWords(entry: StickEntry): string {
    switch (entry.sharing) {
      case "on":
        if (entry.readers === 1) return "↑ Someone is getting it now";
        if (entry.readers > 1) return `↑ ${entry.readers} people are getting it now`;
        return "Nobody is getting it right now.";
      case "starting":
        return "Getting it ready…";
      case "busy":
        return entry.doing ?? "Busy…";
      case "failed":
        return "✕ Stopped: needs you";
      default:
        return "⏸ Stopped by you";
    }
  }

  let empty = $derived(sending.length === 0 && received.length === 0);
  /** A phone's words: it sends from the phone, while OurStick is open
   * (ADR 0029). */
  const phone = onPhone();
</script>

<section class="home">
  {#if empty}
    <div class="welcome">
      <h1>Welcome to OurStick</h1>
      {#if phone}
        <p>
          Send photos, files and folders of any size straight from this phone, and get what others send you. No
          account. Only the people who get the link can see them.
        </p>
      {:else}
        <p>
          Send folders of any size straight from your computer to someone else’s. No account. Only the people you send
          the link to can see them.
        </p>
      {/if}
    </div>
  {/if}

  <div class="verbs">
    <button class="verb primary" onclick={onSend}>
      <span class="arrow" aria-hidden="true">↑</span>
      <span class="title">Send</span>
      <span class="what">{phone ? "Choose photos, files or a folder, get a link to send." : "Choose folders, get a link to send."}</span>
    </button>
    <button class="verb" onclick={onReceive}>
      <span class="arrow" aria-hidden="true">↓</span>
      <span class="title">Receive</span>
      <span class="what">Someone sent you a link? Open it here.</span>
    </button>
  </div>

  {#if sending.length > 0}
    <h2>You are sending</h2>
    <ul>
      {#each sending as entry (entry.id)}
        <li>
          <button class="row-button" onclick={() => onOpenSending(entry.id)}>
            <span class="text">
              <bdi class="name">{entry.name}</bdi>
              <span class="state muted" class:error={entry.sharing === "failed"}>{sendingWords(entry)}</span>
            </span>
            <span class="more" aria-hidden="true">›</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if received.length > 0}
    <h2>You received</h2>
    <ul>
      {#each received as entry (entry.id)}
        <li>
          <button class="row-button" onclick={() => onOpenReceived(entry.id)}>
            <span class="text">
              <!-- The sender's name, isolated: its direction never spills over. -->
              <bdi class="name">{entry.name}</bdi>
              <span class="state muted" class:error={entry.state === "stopped"}>{receivedShort(entry)}</span>
            </span>
            <span class="more" aria-hidden="true">›</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .home {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
    overflow-y: auto;
  }
  .welcome h1 {
    margin: 0 0 6px;
  }
  .welcome p {
    margin: 0;
  }
  .verbs {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 12px;
  }
  .verb {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    height: auto;
    min-height: 110px;
    padding: 16px 18px;
    text-align: left;
    border-radius: var(--radius);
    cursor: pointer;
  }
  .verb:not(.primary) {
    color: var(--accent);
  }
  .verb .arrow {
    font-size: 1.8em;
    line-height: 1;
  }
  .verb .title {
    font-size: 1.4em;
    font-weight: 700;
  }
  .verb .what {
    font-size: 0.95em;
  }
  .verb:not(.primary) .what {
    color: var(--muted);
  }
  h2 {
    margin: 4px 0 0;
    font-size: 0.85em;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row-button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    height: auto;
    padding: 10px 14px;
    text-align: left;
    border-radius: var(--radius);
    cursor: pointer;
  }
  .text {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .more {
    font-size: 1.4em;
    color: var(--muted);
  }
</style>
