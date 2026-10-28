/**
 * Old spellings that browsers built on ICU (Chrome, Edge, Safari) still report,
 * from `resolvedOptions().timeZone` and `supportedValuesOf("timeZone")`, with
 * their current IANA names. The server accepts only the current name ("That is
 * not a valid time zone" otherwise), so a reader in Kolkata or Kyiv could not
 * sign up. These are all such names in Chromium's list (checked against the
 * server on 2026-10-28); `Intl` accepts the new names too.
 */
const CURRENT_ZONE_NAMES: Readonly<Record<string, string>> = {
  "Africa/Asmera": "Africa/Asmara",
  "America/Buenos_Aires": "America/Argentina/Buenos_Aires",
  "America/Catamarca": "America/Argentina/Catamarca",
  "America/Cordoba": "America/Argentina/Cordoba",
  "America/Godthab": "America/Nuuk",
  "America/Indianapolis": "America/Indiana/Indianapolis",
  "America/Jujuy": "America/Argentina/Jujuy",
  "America/Louisville": "America/Kentucky/Louisville",
  "America/Mendoza": "America/Argentina/Mendoza",
  "Asia/Calcutta": "Asia/Kolkata",
  "Asia/Katmandu": "Asia/Kathmandu",
  "Asia/Rangoon": "Asia/Yangon",
  "Asia/Saigon": "Asia/Ho_Chi_Minh",
  "Atlantic/Faeroe": "Atlantic/Faroe",
  "Europe/Kiev": "Europe/Kyiv",
  "Pacific/Enderbury": "Pacific/Kanton",
  "Pacific/Ponape": "Pacific/Pohnpei",
  "Pacific/Truk": "Pacific/Chuuk",
};

/** The zone's current IANA name, the spelling the server stores. */
export function currentZoneName(zone: string): string {
  return CURRENT_ZONE_NAMES[zone] ?? zone;
}

/** The device's IANA time zone, which sign-up stores as the personal zone. */
export function browserTimeZone(): string | null {
  try {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return zone ? currentZoneName(zone) : null;
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
  const all = new Set(zones.map(currentZoneName));
  if (current) all.add(current);
  all.add("UTC");
  return [...all].sort((a, b) => a.localeCompare(b));
}
