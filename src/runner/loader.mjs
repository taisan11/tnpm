// Installed via Node's synchronous hooks so both import and require are covered.
import { registerHooks } from 'node:module';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, extname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const executable = process.env.TNPM_RUNNER_EXECUTABLE;
delete process.env.TNPM_RUNNER_EXECUTABLE;

function bridge(args) {
  const result = spawnSync(executable, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || `tnpm runner bridge failed (${result.status})`);
  return result.stdout;
}

function format(path, context) {
  if (path.endsWith('.mts') || path.endsWith('.mjs')) return 'module';
  if (path.endsWith('.cts') || path.endsWith('.cjs')) return 'commonjs';
  if (context.format === 'module' || context.format === 'module-typescript') return 'module';
  if (context.format === 'commonjs' || context.format === 'commonjs-typescript') return 'commonjs';
  let dir = dirname(path);
  for (;;) {
    try {
      return JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8')).type === 'module' ? 'module' : 'commonjs';
    } catch (error) {
      if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
    }
    const parent = dirname(dir);
    if (parent === dir) return 'commonjs';
    dir = parent;
  }
}

registerHooks({
  resolve(specifier, context, nextResolve) {
    try {
      return nextResolve(specifier, context);
    } catch (error) {
      if (!['ERR_MODULE_NOT_FOUND', 'MODULE_NOT_FOUND', 'ERR_UNSUPPORTED_DIR_IMPORT'].includes(error.code) ||
          !context.parentURL?.startsWith('file:')) throw error;
      try {
        const resolved = bridge(['--tnpm-runner-resolve', fileURLToPath(context.parentURL), specifier, ...context.conditions]);
        return { url: pathToFileURL(resolved).href, shortCircuit: true };
      } catch {
        throw error;
      }
    }
  },
  load(url, context, nextLoad) {
    if (!url.startsWith('file:')) return nextLoad(url, context);
    const path = fileURLToPath(url);
    if (!['.ts', '.tsx', '.mts', '.cts', '.jsx'].includes(extname(path))) return nextLoad(url, context);
    const moduleFormat = format(path, context);
    return { format: moduleFormat, source: bridge(['--tnpm-runner-transform', path, moduleFormat]), shortCircuit: true };
  },
});
//# sourceURL=tnpm-runner-loader.mjs
