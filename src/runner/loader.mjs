// Installed via Node's synchronous hooks so both import and require are covered.
import * as nodeModule from 'node:module';
import { spawn, spawnSync } from 'node:child_process';
import { readFileSync, readSync, writeSync } from 'node:fs';
import { dirname, extname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

if (typeof nodeModule.registerHooks !== 'function') {
  throw new Error(
    `tnpm runner requires Node.js 22.15 or later (found ${process.version})`,
  );
}
if (typeof nodeModule.enableCompileCache === 'function') {
  try {
    nodeModule.enableCompileCache();
  } catch {
    // Compile cache is an optional optimization.
  }
}

const executable = process.env.TNPM_RUNNER_EXECUTABLE;
const configuration = process.env.TNPM_RUNNER_CONFIG;
const projectRoot = JSON.parse(configuration).root;
delete process.env.TNPM_RUNNER_EXECUTABLE;
delete process.env.TNPM_RUNNER_CONFIG;
const remoteSources = new Map();
const resolutions = new Map();
const packages = new Map();
const formats = new Map();

// One long-lived bridge avoids a process launch per module. Linux/macOS expose
// the pipe descriptors; other platforms fall back to one process per request.
let bridge = null;
let useBridge = process.platform !== 'win32';
let bridgeFds = null;

function spawnFallback(args) {
  const result = spawnSync(executable, args, {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, TNPM_RUNNER_CONFIG: configuration },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      result.stderr.trim() || `tnpm runner bridge failed (${result.status})`,
    );
  }
  if (result.stderr) process.stderr.write(result.stderr);
  return result.stdout;
}

function startBridge() {
  if (bridge) return;
  const child = spawn(executable, ['--tnpm-runner-bridge'], {
    stdio: ['pipe', 'pipe', 'inherit'],
    env: { ...process.env, TNPM_RUNNER_CONFIG: configuration },
  });
  const input = child.stdin && child.stdin._handle && child.stdin._handle.fd;
  const output = child.stdout && child.stdout._handle && child.stdout._handle.fd;
  if (typeof input !== 'number' || typeof output !== 'number') {
    child.kill();
    throw new Error('runner bridge descriptors are unavailable');
  }
  bridge = child;
  bridgeFds = { input, output };
  // The bridge is used through raw descriptors; do not let its streams keep the
  // Node event loop alive after the script finishes.
  child.unref();
  child.stdin.unref?.();
  child.stdout.unref?.();
  child.on('exit', () => {
    bridge = null;
    bridgeFds = null;
  });
}

function pause(milliseconds) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
}

function writeAll(fd, buffer) {
  let offset = 0;
  while (offset < buffer.length) {
    try {
      const written = writeSync(fd, buffer, offset, buffer.length - offset);
      if (written <= 0) throw new Error('runner bridge write made no progress');
      offset += written;
    } catch (error) {
      if (error.code === 'EAGAIN') {
        pause(1);
        continue;
      }
      if (error.code === 'EINTR') continue;
      throw error;
    }
  }
}

function readExact(fd, length) {
  const buffer = Buffer.allocUnsafe(length);
  let offset = 0;
  while (offset < length) {
    let count;
    try {
      count = readSync(fd, buffer, offset, length - offset, null);
    } catch (error) {
      if (error.code === 'EAGAIN') {
        pause(1);
        continue;
      }
      if (error.code === 'EINTR') continue;
      throw error;
    }
    if (count === 0) throw new Error('runner bridge closed unexpectedly');
    offset += count;
  }
  return buffer;
}

function exchange(request) {
  try {
    startBridge();
    const payload = Buffer.from(JSON.stringify(request));
    const header = Buffer.allocUnsafe(4);
    header.writeUInt32LE(payload.length, 0);
    writeAll(bridgeFds.input, header);
    writeAll(bridgeFds.input, payload);
    const length = readExact(bridgeFds.output, 4).readUInt32LE(0);
    return JSON.parse(readExact(bridgeFds.output, length).toString('utf8'));
  } catch (error) {
    error.bridgeTransport = true;
    throw error;
  }
}

function legacyRequest(request) {
  switch (request.op) {
    case 'transform':
      return {
        ok: true,
        code: spawnFallback([
          '--tnpm-runner-transform',
          request.path,
          request.format,
        ]),
      };
    case 'resolve':
      return {
        ok: true,
        path: spawnFallback([
          '--tnpm-runner-resolve',
          request.parent,
          request.specifier,
          ...request.conditions,
        ]),
      };
    case 'package':
      return {
        ok: true,
        path: spawnFallback([
          '--tnpm-runner-package',
          request.specifier,
          request.parent,
          ...request.conditions,
        ]),
      };
    case 'fetch':
      return {
        ok: true,
        value: JSON.parse(spawnFallback(['--tnpm-runner-fetch', request.url])),
      };
    default:
      throw new Error(`unknown runner request: ${request.op}`);
  }
}

function call(request) {
  if (useBridge) {
    try {
      const response = exchange(request);
      if (response.ok === false) throw new Error(response.error);
      return response;
    } catch (error) {
      if (!error.bridgeTransport) throw error;
      // The bridge died or could not be started; use the per-request process.
      useBridge = false;
      if (bridge) {
        try {
          bridge.kill();
        } catch {
          // Ignore cleanup failures.
        }
        bridge = null;
        bridgeFds = null;
      }
    }
  }
  const response = legacyRequest(request);
  if (response.ok === false) throw new Error(response.error);
  return response;
}

function isRemote(url) {
  return typeof url === 'string' && /^https?:/i.test(url);
}

function remote(url) {
  let source = remoteSources.get(url);
  if (!source) {
    source = call({ op: 'fetch', url });
    source = source.value;
    remoteSources.set(url, source);
    remoteSources.set(source.url, source);
  }
  return source;
}

function packageFormat(specifier, parent, conditions) {
  const key = `${parent}\0${specifier}\0${conditions.join(',')}`;
  let path = packages.get(key);
  if (path === undefined) {
    path = call({ op: 'package', specifier, parent, conditions }).path;
    packages.set(key, path);
  }
  return path;
}

function fallbackResolution(parent, specifier, conditions) {
  const key = `${parent}\0${specifier}\0${conditions.join(',')}`;
  let path = resolutions.get(key);
  if (path === undefined) {
    path = call({ op: 'resolve', parent, specifier, conditions }).path;
    resolutions.set(key, path);
  }
  return path;
}

function format(path, context) {
  if (path.endsWith('.mts') || path.endsWith('.mjs')) return 'module';
  if (path.endsWith('.cts') || path.endsWith('.cjs')) return 'commonjs';
  if (context.format === 'module' || context.format === 'module-typescript') return 'module';
  if (context.format === 'commonjs' || context.format === 'commonjs-typescript') return 'commonjs';
  const start = dirname(path);
  const cached = formats.get(start);
  if (cached) return cached;
  let dir = start;
  for (;;) {
    try {
      const type = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8')).type;
      const result = type === 'module' ? 'module' : 'commonjs';
      formats.set(start, result);
      return result;
    } catch (error) {
      if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
    }
    const parent = dirname(dir);
    if (parent === dir) {
      formats.set(start, 'commonjs');
      return 'commonjs';
    }
    dir = parent;
  }
}

nodeModule.registerHooks({
  resolve(specifier, context, nextResolve) {
    const conditions = context.conditions || [];
    if (specifier.startsWith('jsr:') || specifier.startsWith('npm:')) {
      const parent = context.parentURL?.startsWith('file:')
        ? fileURLToPath(context.parentURL)
        : join(projectRoot, '.tnpm-runner-entry.mjs');
      const path = packageFormat(specifier, parent, conditions);
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
        const resolved = fallbackResolution(fileURLToPath(context.parentURL), specifier, conditions);
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
    return {
      format: moduleFormat,
      source: call({ op: 'transform', path, format: moduleFormat }).code,
      shortCircuit: true,
    };
  },
});

process.on('exit', () => {
  if (bridge) {
    try {
      bridge.kill();
    } catch {
      // Ignore cleanup failures.
    }
  }
});
//# sourceURL=tnpm-runner-loader.mjs
