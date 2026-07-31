export const UPSTREAM_REPOSITORY = 'https://github.com/P4suta/aozora';

export function normalizeRepository(remote: string): string {
  return remote
    .replace(/^git@github\.com:/, 'https://github.com/')
    .replace(/^ssh:\/\/git@github\.com\//, 'https://github.com/')
    .replace(/\.git$/, '');
}

export function assertCanonicalRepository(remote: string): void {
  if (normalizeRepository(remote) !== UPSTREAM_REPOSITORY) {
    throw new Error(
      `canonical playground-ui must come from ${UPSTREAM_REPOSITORY}`,
    );
  }
}
