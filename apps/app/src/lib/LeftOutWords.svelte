<script lang="ts">
  // What a received stick leaves out, grouped by the folder it is inside
  // (ADR 0012, Amendment 1): "Films; MUSIC and Pictures (inside Disk)".
  // Every name is the sender's: escaped already, and isolated here.
  let {
    list,
  }: {
    list: { shown: string; inside: string | null }[];
  } = $props();

  /** At most this many names of a group, then "and N more". */
  const SHOWN = 3;

  let groups = $derived.by(() => {
    const byFolder = new Map<string | null, string[]>();
    for (const item of list) {
      const names = byFolder.get(item.inside) ?? [];
      names.push(item.shown);
      byFolder.set(item.inside, names);
    }
    return [...byFolder].map(([inside, names]) => ({ inside, names }));
  });
</script>

{#each groups as group, index (group.inside ?? "")}{index > 0 ? "; " : ""}{#each group.names.slice(0, SHOWN) as name, at (at)}{at === 0
      ? ""
      : at === group.names.length - 1
        ? " and "
        : ", "}<bdi>{name}</bdi>{/each}{group.names.length > SHOWN
    ? ` and ${group.names.length - SHOWN} more`
    : ""}{#if group.inside !== null}{" "}(inside <bdi>{group.inside}</bdi>){/if}{/each}
