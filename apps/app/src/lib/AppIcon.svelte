<script lang="ts">
  // OurStick's own icon (src-tauri/icons/source/icon.svg): the blue stick
  // with the arrow up. Its gradients get ids of their own on each use.
  let { size = 32 }: { size?: number } = $props();
  const id = `app-icon-${Math.random().toString(36).slice(2, 9)}`;
</script>

<svg width={size} height={size} viewBox="0 0 512 512" aria-hidden="true" style="flex-shrink: 0">
  <defs>
    <linearGradient id="{id}-g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#4a95ff" /><stop offset="1" stop-color="#0a5fd8" />
    </linearGradient>
    <linearGradient id="{id}-c" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff" /><stop offset="1" stop-color="#e4e4e8" />
    </linearGradient>
  </defs>
  <rect x="146" y="392" width="220" height="104" rx="12" fill="url(#{id}-c)" stroke="#b9b9c0" stroke-width="4" />
  <rect x="180" y="424" width="50" height="28" rx="5" fill="#8d8d96" />
  <rect x="282" y="424" width="50" height="28" rx="5" fill="#8d8d96" />
  <rect x="106" y="16" width="300" height="392" rx="60" fill="url(#{id}-g)" stroke="#0a52b8" stroke-width="4" />
  <rect x="112" y="22" width="288" height="160" rx="54" fill="#ffffff" opacity="0.10" />
  <g fill="none" stroke="#ffffff" stroke-width="30" stroke-linecap="round" stroke-linejoin="round">
    <path d="M256 300 V120" /><path d="M196 180 L256 120 L316 180" />
  </g>
</svg>
