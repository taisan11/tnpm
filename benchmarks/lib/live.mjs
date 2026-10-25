// Discover once outside timing, then pin every CLI to the same published version.
export async function resolveLiveTarget(invoke, tool, fixture) {
  const name = 'hono';
  const args = [tool === 'bun' ? 'info' : 'view', name, 'version', '--registry', fixture.registries[tool]];
  args.push(tool === 'npm' ? '--prefer-online' : '--no-cache');
  if (tool === 'bun') args.push('--cache-dir', fixture.environments.bun.BUN_INSTALL_CACHE_DIR);
  const { stdout } = await invoke(tool, args);
  let version = stdout.toString().trim();
  // Accept plain output, a JSON scalar, and npm's single-element array form.
  try {
    const value = JSON.parse(version);
    version = Array.isArray(value) && value.length === 1 ? value[0] : value;
  } catch { /* Plain SemVer output is expected from most versions of these CLIs. */ }
  if (typeof version !== 'string' || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`Cannot resolve the published hono version: ${JSON.stringify(version)}`);
  }
  return { name, version, spec: `${name}@${version}`, resolvedBy: tool };
}

export function requestDelta(before, after) {
  return before === null || after === null ? null : after - before;
}
