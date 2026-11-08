// Installed via Node's synchronous hooks so both import and require are covered.
import { registerHooks } from 'node:module';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, extname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const executable = process.env.TNPM_RUNNER_EXECUTABLE;
const configuration = process.env.TNPM_RUNNER_CONFIG;
const projectRoot = JSON.parse(configuration).root;
delete process.env.TNPM_RUNNER_EXECUTABLE;
delete process.env.TNPM_RUNNER_CONFIG;
const remoteSources = new Map();

function bridge(args) {
  const result = spawnSync(executable, args, {
    encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, TNPM_RUNNER_CONFIG: configuration },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || `tnpm runner bridge failed (${result.status})`);
  if (result.stderr) process.stderr.write(result.stderr);
  return result.stdout;
}

function isRemote(url) {
  return typeof url === 'string' && /^https?:/i.test(url);
}

function remote(url) {
  let source = remoteSources.get(url);
  if (!source) {
    source = JSON.parse(bridge(['--tnpm-runner-fetch', url]));
    remoteSources.set(url, source);
    remoteSources.set(source.url, source);
  }
  return source;
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
    if (specifier.startsWith('jsr:') || specifier.startsWith('npm:')) {
      const parent = context.parentURL?.startsWith('file:')
        ? fileURLToPath(context.parentURL) : join(projectRoot, '.tnpm-runner-entry.mjs');
      const path = bridge(['--tnpm-runner-package', specifier, parent, ...context.conditions]);
      return { url: pathToFileURL(path).href, shortCircuit: true };
    }
    if (isRemote(specifier)) {
      if (isRemote(context.parentURL) && new URL(context.parentURL).protocol === 'https:' && new URL(specifier).protocol === 'http:') {
        throw new Error('HTTPS module imports cannot downgrade to HTTP');
      }
      const source = remote(new URL(specifier).href);
      return { url: source.url, format: source.format, shortCircuit: true };
    }
    if (isRemote(context.parentURL)) {
      if (specifier.startsWith('.') || specifier.startsWith('/')) {
        const url = new URL(specifier, context.parentURL);
        if (new URL(context.parentURL).protocol === 'https:' && url.protocol !== 'https:') {
          throw new Error('HTTPS module imports cannot downgrade to HTTP');
        }
        const source = remote(url.href);
        return { url: source.url, format: source.format, shortCircuit: true };
      }
      if (specifier.startsWith('file:')) throw new Error('Remote modules cannot import local file URLs');
      if (!specifier.startsWith('node:') && !specifier.startsWith('data:')) {
        throw new Error(`Bare imports from remote modules require an npm: or jsr: specifier: ${specifier}`);
      }
    }
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
    if (isRemote(url)) {
      const source = remote(url);
      if (source.format === 'json' && context.importAttributes?.type !== 'json') {
        throw new Error('Remote JSON imports require with { type: "json" }');
      }
      return { format: source.format, source: source.source, shortCircuit: true };
    }
    if (!url.startsWith('file:')) return nextLoad(url, context);
    const path = fileURLToPath(url);
    if (!['.ts', '.tsx', '.mts', '.cts', '.jsx'].includes(extname(path))) return nextLoad(url, context);
    const moduleFormat = format(path, context);
    return { format: moduleFormat, source: bridge(['--tnpm-runner-transform', path, moduleFormat]), shortCircuit: true };
  },
});
//# sourceURL=tnpm-runner-loader.mjs
