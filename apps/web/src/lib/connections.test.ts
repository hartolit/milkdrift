import { describe, expect, it, vi } from 'vitest';
import { endpointUrl, loadProfiles, saveProfiles } from './connections';

describe('connection profiles', () => {
  it('still allows connection setup when the browser denies access to saved storage', () => {
    const blocked = vi.spyOn(window, 'localStorage', 'get').mockImplementation(() => { throw new Error('Storage access denied'); });
    try { expect(loadProfiles()).toEqual([]); }
    finally { blocked.mockRestore(); }
  });

  it('saves only named non-secret settings', () => {
    const profile = { id: 'a', cachedName: 'Studio', endpoint: 'https://studio.example', daemonId: 'one', token: 'do-not-save' };
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
    const profiles = Array.from({ length: 17 }, (_, index) => ({ id: String(index), cachedName: `Daemon ${index}`, endpoint: 'https://studio.example', daemonId: String(index) }));
    expect(() => saveProfiles(profiles)).toThrow('At most 16');
  });

  it('adopts an old local label only as a cached name while retaining its stable pin and selection id', () => {
    window.localStorage.setItem('milkdrift.connections.v1', JSON.stringify([{ id: 'selected-before', label: 'Old nickname', endpoint: 'https://studio.example', daemonId: 'stable-pin' }]));
    expect(loadProfiles()).toEqual([{ id: 'selected-before', cachedName: 'Old nickname', endpoint: 'https://studio.example', daemonId: 'stable-pin' }]);
    saveProfiles(loadProfiles());
    expect(window.localStorage.getItem('milkdrift.connections.v1')).not.toContain('label');
  });
});
