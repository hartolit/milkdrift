import { describe, expect, it } from 'vitest';
import { endpointUrl, loadProfiles, saveProfiles } from './connections';

describe('connection profiles', () => {
  it('saves only named non-secret settings', () => {
    const profile = { id: 'a', label: 'Studio', endpoint: 'https://studio.example', daemonId: 'one', token: 'do-not-save' };
    saveProfiles([profile]);
    expect(JSON.stringify(loadProfiles())).not.toContain('do-not-save');
    expect(window.localStorage.getItem('milkdrift.connections.v1')).not.toContain('token');
  });

  it('permits HTTPS and loopback development but refuses mixed content and URL credentials', () => {
    expect(endpointUrl('http://127.0.0.1:8080', 'http:')).toBe('http://127.0.0.1:8080');
    expect(endpointUrl('https://studio.example', 'https:')).toBe('https://studio.example');
    expect(() => endpointUrl('http://192.168.1.2:8080', 'http:')).toThrow('HTTPS');
    expect(() => endpointUrl('http://127.0.0.1:8080', 'https:')).toThrow('HTTPS page');
    expect(() => endpointUrl('https://secret@example.com', 'https:')).toThrow('credentials');
    expect(() => endpointUrl('https://example.com?token=secret', 'https:')).toThrow('query');
  });

  it('refuses saved-profile overflow without silently dropping a connection', () => {
    const profiles = Array.from({ length: 17 }, (_, index) => ({ id: String(index), label: `Daemon ${index}`, endpoint: 'https://studio.example', daemonId: String(index) }));
    expect(() => saveProfiles(profiles)).toThrow('At most 16');
  });
});
