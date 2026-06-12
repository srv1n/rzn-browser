import { describe, expect, it } from 'vitest';
import { normalizeCloudServerUrl } from './cloudTransport';

describe('normalizeCloudServerUrl', () => {
  it('accepts remote https and strips path noise', () => {
    expect(normalizeCloudServerUrl('https://cloud.example.com/?token=x#frag')).toBe('https://cloud.example.com');
  });

  it('accepts loopback http for local development', () => {
    expect(normalizeCloudServerUrl('http://127.0.0.1:8787/')).toBe('http://127.0.0.1:8787');
    expect(normalizeCloudServerUrl('http://localhost:8787/')).toBe('http://localhost:8787');
  });

  it('rejects remote plaintext http', () => {
    expect(() => normalizeCloudServerUrl('http://cloud.example.com')).toThrow(/https/);
  });
});
