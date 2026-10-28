/** The device's IANA time zone, which sign-up stores as the personal zone. */
export function browserTimeZone(): string | null {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

/** The zone a post's time shows in: its club's for a club post, the viewer's for a diary entry. */
export function postTimeZone(
  post: { club: { slug: string } | null },
  user: { timezone: string; clubs: { slug: string; timezone: string }[] } | null,
): string {
  if (post.club) {
    const club = user?.clubs.find((row) => row.slug === post.club!.slug);
    if (club) return club.timezone;
  }
  return user?.timezone || "UTC";
}

/** The IANA zones this browser knows, for a time-zone picker; `current` is always among them. */
export function timeZoneOptions(current?: string | null): string[] {
  let zones: string[] = [];
  try {
    const intl = Intl as unknown as { supportedValuesOf?: (key: string) => string[] };
    zones = intl.supportedValuesOf?.("timeZone") ?? [];
  } catch {
    zones = [];
  }
  const all = new Set(zones);
  if (current) all.add(current);
  all.add("UTC");
  return [...all].sort((a, b) => a.localeCompare(b));
}
