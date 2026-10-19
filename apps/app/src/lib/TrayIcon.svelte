<script lang="ts">
  // The small one-colour icon near the clock (src-tauri/icons/source/tray.svg).
  let { size = 16, color = "#1b1f24" }: { size?: number; color?: string } = $props();
  const id = `tray-icon-${Math.random().toString(36).slice(2, 9)}`;
</script>

<svg width={size} height={size} viewBox="0 0 64 64" aria-hidden="true" style="flex-shrink: 0">
  <defs>
    <mask {id}>
      <rect width="64" height="64" fill="#fff" />
      <g fill="none" stroke="#000" stroke-width="4.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M32 37 V15" /><path d="M24.5 22.5 L32 15 L39.5 22.5" />
      </g>
      <rect x="25" y="51" width="5" height="4" fill="#000" />
      <rect x="34" y="51" width="5" height="4" fill="#000" />
    </mask>
  </defs>
  <g mask="url(#{id})" fill={color}>
    <rect x="17" y="3" width="30" height="44" rx="7" /><rect x="21" y="47" width="22" height="13" rx="2" />
  </g>
</svg>
