/// Espacenet bibliographic page for a publication, or null when the
/// publication key is not in `<country><number>` form.
export function espacenetUrl(pubKey: string, kindCodes: string[]): string | null {
  const match = pubKey.match(/^([A-Za-z]+)(\d+)$/);
  if (!match) return null;
  const [, country, number] = match;
  const kind = kindCodes[0] ?? "";
  return `https://worldwide.espacenet.com/publicationDetails/biblio?CC=${country}&NR=${number}${kind}&KC=${kind}&FT=D`;
}
