<script lang="ts">
  // A picture of this computer's screen (ADR 0028 §6): where OurStick's
  // picture is, marked "Click here", and the small icon near the clock,
  // "Or here". Linux without a tray host: the ⊞ key and the window among the
  // others. A drawing in fixed colours: it shows a screen, not the theme.
  import AppIcon from "./AppIcon.svelte";
  import TrayIcon from "./TrayIcon.svelte";

  let {
    platform,
    tray,
    height = 170,
  }: { platform: string; tray: boolean; height?: number } = $props();
</script>

{#snippet other(color: string, size = 30)}
  <span class="other" style="width: {size}px; height: {size}px; background: {color}"></span>
{/snippet}

{#snippet app(size = 30)}
  <span class="ring app" style="width: {size}px; height: {size}px"><AppIcon size={size} /></span>
{/snippet}

{#if platform === "windows"}
  <div
    class="picture"
    style="height: {height}px"
    role="img"
    aria-label="The bottom of the screen in Windows. The OurStick picture is in the middle. A small OurStick icon is also near the clock, or under the ^ button."
  >
    <div class="sky" style="background: linear-gradient(160deg, #b9d3f0, #e3ecf7)"></div>
    <span class="callout" style="left: 130px; bottom: 66px">Click here</span>
    {#if tray}<span class="callout" style="left: 271px; bottom: 66px">Or here</span>{/if}
    <div class="taskbar">
      <span class="grow"></span>
      <span class="icons">{@render other("#3b82f6")}{@render other("#f2c94c")}{@render app()}{@render other("#9aa3ad")}</span>
      <span class="grow"></span>
      <span class="icons clock">
        <span class="caret" class:ring={tray}>^</span>
        <span aria-hidden="true">📶 🔊</span>
        <span class="time"><span>12:40</span><span>19/10/2026</span></span>
      </span>
    </div>
  </div>
{:else if platform === "macos"}
  <div
    class="picture"
    style="height: {height}px"
    role="img"
    aria-label="A Mac screen. The OurStick picture is at the bottom of the screen. A small OurStick icon is also at the top, near the clock."
  >
    <div class="sky" style="background: linear-gradient(160deg, #c9b8e8, #f0e6f7)"></div>
    <div class="menubar">
      <b>Finder</b><span>File</span><span>Edit</span>
      <span class="grow"></span>
      <span class="ring flat"><TrayIcon size={16} /></span>
      <span>Mon 19 Oct 12:40</span>
    </div>
    <span class="callout" style="left: 270px; top: 40px">Or here</span>
    <span class="callout" style="left: 26px; bottom: 26px">Click here →</span>
    <div class="dock">
      {@render other("#3b82f6", 34)}{@render other("#34c759", 34)}
      <span class="running">{@render app(34)}<span class="dot"></span></span>
      {@render other("#ff9500", 34)}
    </div>
  </div>
{:else if tray}
  <div
    class="picture"
    style="height: {height}px"
    role="img"
    aria-label="An Ubuntu screen. The OurStick picture is on the left of the screen. A small OurStick icon is also at the top right."
  >
    <div class="sky" style="background: linear-gradient(160deg, #e8a87c, #f6dcc8)"></div>
    <div class="topbar">
      <span class="grow center">Oct 19 12:40</span>
      <span class="icons"><span class="ring flat light"><TrayIcon size={16} color="#ffffff" /></span><span aria-hidden="true">📶 🔊 ⏻</span></span>
    </div>
    <div class="side">{@render other("#3b82f6")}{@render app()}{@render other("#e95420")}</div>
    <span class="callout" style="left: 72px; top: 76px">Click here</span>
    <span class="callout" style="right: 66px; top: 44px">Or here</span>
  </div>
{:else}
  <div
    class="picture"
    style="height: {height}px; background: #2b2b2b"
    role="img"
    aria-label="Press the key with the Windows logo. All open windows appear. Click the OurStick window."
  >
    <div class="key-step">
      <span class="key">⊞</span><span>Press this key</span>
    </div>
    <span class="then" aria-hidden="true">→</span>
    <div class="windows">
      <span class="window-shape"></span>
      <span class="window-ours"><span class="ring shot"><AppIcon size={44} /></span>OurStick</span>
      <span class="window-shape"></span>
    </div>
  </div>
{/if}

<style>
  .picture {
    position: relative;
    flex-shrink: 0;
    border-radius: 10px;
    overflow: hidden;
    color: #1b1f24;
    font-size: 13px;
  }
  .sky {
    position: absolute;
    inset: 0;
  }
  .grow {
    flex-grow: 1;
  }
  .center {
    text-align: center;
    font-weight: 600;
  }
  .icons {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .other {
    border-radius: 8px;
    flex-shrink: 0;
  }
  .app {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  /* Dark orange: at least 3:1 against every light background here. */
  .ring {
    outline: 3px solid #b45309;
    outline-offset: 3px;
    border-radius: 6px;
  }
  .ring.light {
    outline-color: #f59e0b;
  }
  .flat {
    display: flex;
  }
  .callout {
    position: absolute;
    background: #b45309;
    color: #ffffff;
    font-size: 14px;
    font-weight: 700;
    padding: 4px 9px;
    border-radius: 6px;
    white-space: nowrap;
  }
  .taskbar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 50px;
    display: flex;
    align-items: center;
    padding: 0 12px;
    background: #f3f3f3;
    border-top: 1px solid #d0d0d0;
  }
  .clock {
    gap: 12px;
  }
  .caret {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    background: #e3e3e3;
    font-weight: 700;
  }
  .time {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    line-height: 1.2;
  }
  .menubar,
  .topbar {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 12px;
  }
  .menubar {
    background: rgb(245 245 247 / 95%);
  }
  .topbar {
    background: #1d1d1d;
    color: #ffffff;
  }
  .dock {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    bottom: 8px;
    display: flex;
    align-items: flex-end;
    gap: 10px;
    padding: 7px 12px 10px;
    background: rgb(255 255 255 / 60%);
    border: 1px solid rgb(255 255 255 / 80%);
    border-radius: 16px;
  }
  .running {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .dot {
    width: 4px;
    height: 4px;
    border-radius: 2px;
    background: #1b1f24;
  }
  .side {
    position: absolute;
    left: 0;
    top: 30px;
    bottom: 0;
    width: 52px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding-top: 12px;
    background: rgb(29 29 29 / 85%);
  }
  .key-step {
    position: absolute;
    left: 24px;
    top: 50%;
    transform: translateY(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    color: #ffffff;
  }
  .key {
    min-width: 54px;
    height: 54px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 9px;
    background: #ffffff;
    border: 1px solid #c9ced6;
    box-shadow: 0 3px 0 #c9ced6;
    font-size: 26px;
    color: #1b1f24;
  }
  .then {
    position: absolute;
    left: 116px;
    top: 50%;
    transform: translateY(-50%);
    color: #ffffff;
    font-size: 26px;
  }
  .windows {
    position: absolute;
    left: 160px;
    right: 20px;
    top: 26px;
    bottom: 26px;
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .window-shape {
    flex: 1;
    height: 80px;
    border-radius: 6px;
    background: #4a4a4a;
  }
  .window-ours {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    color: #ffffff;
    font-weight: 600;
  }
  .shot {
    width: 100%;
    height: 80px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #f4f5f7;
  }
</style>
