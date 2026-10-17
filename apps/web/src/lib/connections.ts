/** A saved pin identifies the daemon; its cached name is only the last observed presentation. */
export interface ConnectionProfile {
  id: string;
  cachedName: string;
  endpoint: string;
  daemonId: string;
}

const profileKey = 'milkdrift.connections.v1';
export const maxConnections = 16;

export function endpointUrl(value: string, pageProtocol = window.location.protocol): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error('Enter the complete daemon URL, including https:// or loopback http://.');
  }
  if (url.username || url.password || url.search || url.hash || url.pathname !== '/') {
    throw new Error('Use a daemon origin without credentials, a path, query parameters or a fragment.');
  }
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback)) {
    throw new Error('LAN and phone connections require browser-trusted HTTPS. HTTP is allowed only for loopback development.');
  }
  if (pageProtocol === 'https:' && url.protocol === 'http:') {
    throw new Error('This HTTPS page cannot connect to an HTTP daemon. Use an HTTPS daemon endpoint.');
  }
  return url.origin;
}

export function loadProfiles(storage?: Storage): ConnectionProfile[] {
  try {
    const value: unknown = JSON.parse((storage ?? window.localStorage).getItem(profileKey) ?? '[]');
    if (!Array.isArray(value)) return [];
    return value.slice(0, maxConnections).filter((item) =>
      item !== null && typeof item === 'object' &&
      typeof item.id === 'string' && (typeof item.cachedName === 'string' || typeof item.label === 'string') &&
      typeof item.endpoint === 'string' && typeof item.daemonId === 'string',
    ).map(({ id, cachedName, label, endpoint, daemonId }) => ({ id, cachedName: typeof cachedName === 'string' ? cachedName : label, endpoint, daemonId }));
  } catch {
    return [];
  }
}

export function saveProfiles(profiles: ConnectionProfile[], storage: Storage = window.localStorage): void {
  if (profiles.length > maxConnections) throw new Error(`At most ${maxConnections} daemon connections can be saved. Forget a connection before adding another.`);
  storage.setItem(profileKey, JSON.stringify(profiles.map(({ id, cachedName, endpoint, daemonId }) =>
    ({ id, cachedName, endpoint, daemonId }),
  )));
}
