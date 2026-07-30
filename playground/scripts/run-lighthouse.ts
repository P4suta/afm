export {};

for (const config of ['.lighthouserc.cjs', '.lighthouserc.mobile.cjs']) {
  const child = Bun.spawn(
    ['bun', 'x', '--no-install', 'lhci', 'autorun', '--config', config],
    {
      stdin: 'inherit',
      stdout: 'inherit',
      stderr: 'inherit',
    },
  );
  const exitCode = await child.exited;
  if (exitCode !== 0) globalThis.process.exit(exitCode);
}
