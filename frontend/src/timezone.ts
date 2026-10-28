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
