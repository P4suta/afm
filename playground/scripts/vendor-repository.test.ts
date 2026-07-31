import { describe, expect, it } from 'vitest';

import {
  assertCanonicalRepository,
  normalizeRepository,
  UPSTREAM_REPOSITORY,
} from './vendor-repository';

describe('canonical playground UI repository identity', () => {
  it.each([
    'git@github.com:P4suta/aozora.git',
    'ssh://git@github.com/P4suta/aozora.git',
    'https://github.com/P4suta/aozora',
    'https://github.com/P4suta/aozora.git',
  ])('normalizes %s', (remote) => {
    expect(normalizeRepository(remote)).toBe(UPSTREAM_REPOSITORY);
    expect(() => assertCanonicalRepository(remote)).not.toThrow();
  });

  it('rejects a fork even when its package bytes happen to match', () => {
    expect(() =>
      assertCanonicalRepository('git@github.com:someone-else/aozora.git'),
    ).toThrow(`canonical playground-ui must come from ${UPSTREAM_REPOSITORY}`);
  });
});
