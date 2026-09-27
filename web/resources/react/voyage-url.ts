// Presentation identity only. The Vessel catalogue/snapshot and scoped ticket remain authoritative.
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export type VoyageLocation = {vessel: string; session: string};
export function voyageLocation(pathname: string): VoyageLocation | null {
    const parts = pathname.split('/');
    if (parts.length !== 4 || parts[0] !== '' || parts[1] !== 'voyages' || !uuid.test(parts[2]) || !uuid.test(parts[3])) return null;
    return {vessel: parts[2].toLowerCase(), session: parts[3].toLowerCase()};
}
export function voyagePath(vessel: string, session: string): string {
    if (!uuid.test(vessel) || !uuid.test(session)) throw new Error('Invalid voyage location');
    return `/voyages/${vessel.toLowerCase()}/${session.toLowerCase()}`;
}
