/// Orders tags depth-first by parent, siblings by name, for tree display.
/// The parent hierarchy is organisational only (see docs/DECISIONS.md).
/// A tag whose parent is not in `items` is shown at the top level.
export function treeOrder<T>(
  items: T[],
  id: (item: T) => number,
  parentId: (item: T) => number | null,
  name: (item: T) => string,
): { item: T; depth: number }[] {
  const ids = new Set(items.map(id));
  const children = new Map<number | null, T[]>();
  for (const item of items) {
    const parent = parentId(item);
    const key = parent !== null && ids.has(parent) ? parent : null;
    children.set(key, [...(children.get(key) ?? []), item]);
  }
  for (const list of children.values()) list.sort((a, b) => name(a).localeCompare(name(b)));

  const ordered: { item: T; depth: number }[] = [];
  const visit = (parent: number | null, depth: number) => {
    for (const item of children.get(parent) ?? []) {
      ordered.push({ item, depth });
      visit(id(item), depth + 1);
    }
  };
  visit(null, 0);
  return ordered;
}
