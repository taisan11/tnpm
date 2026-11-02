import { createHash } from 'node:crypto';
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, posix } from 'node:path';
import { TOOLS } from './fixtures.mjs';
import { execute } from './process.mjs';

export const TEMPLATE = 'templates/cloudflare-workers/';
export const SOURCE = 'https://github.com/honojs/starter/tree/main/templates/cloudflare-workers';

export function templateFiles(tree) {
  if (tree.truncated) throw new Error('GitHub tree was truncated');
  return tree.tree.filter(entry => entry.type === 'blob' && entry.path.startsWith(TEMPLATE)).map(entry => {
    const path = entry.path.slice(TEMPLATE.length);
    if (!path || path.includes('\\') || posix.normalize(path) !== path || path.startsWith('/') || path.split('/').some(p => p === '..' || p === '.')) throw new Error(`Unsafe template path: ${path}`);
    if (!['100644', '100755'].includes(entry.mode)) throw new Error(`Unsupported template file mode: ${entry.mode}`);
    return { path, sha: entry.sha };
  });
}

export function installArgs(tool, offline = false) {
  return ['install', ...(offline ? ['--offline'] : tool === 'bun' ? ['--prefer-offline'] : [])];
}

export function isolatedEnvironment(home, inherited = process.env) {
  const env = Object.fromEntries(Object.entries(inherited).filter(([key]) => !/^(npm_|bun_|tnpm_|xdg_)/i.test(key)
    && !/^(node_options|node_extra_ca_certs|node_env|ci)$/i.test(key) && !/(^|_)proxy$/i.test(key)));
  return { ...env, HOME: home, USERPROFILE: home, NO_COLOR: '1', FORCE_COLOR: '0', CI: '1',
    XDG_CONFIG_HOME: join(home, '.config'), XDG_CACHE_HOME: join(home, '.cache'),
    BUN_INSTALL: join(home, '.bun'), BUN_INSTALL_CACHE_DIR: join(home, '.bun/install/cache'),
    npm_config_userconfig: join(home, 'user.npmrc'), npm_config_globalconfig: join(home, 'global.npmrc'),
    npm_config_cache: join(home, '.npm'), npm_config_registry: 'https://registry.npmjs.org/',
    npm_config_update_notifier: 'false', npm_config_audit: 'false', npm_config_fund: 'false' };
}

async function github(url) {
  const response = await fetch(url, { headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'tnpm-install-benchmark' }, signal: AbortSignal.timeout(60_000) });
  if (!response.ok) throw new Error(`GitHub ${response.status}: ${url}`);
  return response;
}

export async function installedPackages(project) {
  const installed = [];
  async function visit(modules) {
    let entries;
    try { entries = await readdir(modules, { withFileTypes: true }); } catch (e) { if (e.code === 'ENOENT') return; throw e; }
    for (const entry of entries) {
      if (entry.name.startsWith('.')) continue;
      if (entry.isSymbolicLink()) throw new Error(`Unexpected package symlink: ${join(modules, entry.name)}`);
      if (entry.name.startsWith('@')) { await visit(join(modules, entry.name)); continue; }
      const directory = join(modules, entry.name);
      const manifest = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'));
      if (!manifest.name || !manifest.version) throw new Error(`Invalid installed manifest: ${directory}`);
      installed.push(`${manifest.name}@${manifest.version}`);
      await visit(join(directory, 'node_modules'));
    }
  }
  await visit(join(project, 'node_modules'));
  return installed.sort();
}

export async function createInstallFixture(binaries, ref = 'main') {
  const parent = join(tmpdir(), 'opencode');
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(join(parent, 'tnpm-install-'));
  try {
    const commit = await (await github(`https://api.github.com/repos/honojs/starter/commits/${encodeURIComponent(ref)}`)).json();
    if (!/^[a-f0-9]{40}$/.test(commit.sha)) throw new Error('GitHub did not return a commit SHA');
    const tree = await (await github(`https://api.github.com/repos/honojs/starter/git/trees/${commit.sha}?recursive=1`)).json();
    const files = templateFiles(tree);
    if (!files.some(file => file.path === 'package.json')) throw new Error('Template has no package.json');
    const template = join(root, 'template');
    await Promise.all(files.map(async file => {
      const bytes = Buffer.from(await (await github(`https://raw.githubusercontent.com/honojs/starter/${commit.sha}/${TEMPLATE}${file.path}`)).arrayBuffer());
      const sha = createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
      if (sha !== file.sha) throw new Error(`Template Git blob checksum mismatch: ${file.path}`);
      await mkdir(dirname(join(template, file.path)), { recursive: true });
      await writeFile(join(template, file.path), bytes);
    }));
    const packageBytes = await readFile(join(template, 'package.json'));
    const manifest = JSON.parse(packageBytes);
    const projects = {};
    const environments = {};
    for (const tool of [...TOOLS, 'bootstrap']) {
      const home = join(root, 'homes', tool);
      await mkdir(home, { recursive: true });
      await writeFile(join(home, 'user.npmrc'), '');
      await writeFile(join(home, 'global.npmrc'), '');
      environments[tool] = isolatedEnvironment(home);
      projects[tool] = join(root, tool);
      await cp(template, projects[tool], { recursive: true });
    }
    // Pin the entire graph once. Bun imports this into its native lock outside timing.
    await execute(binaries.npm, ['install', '--package-lock-only', '--ignore-scripts', '--no-audit', '--no-fund'],
      { cwd: projects.bootstrap, env: environments.bootstrap, timeout: 180_000 });
    const lockBytes = await readFile(join(projects.bootstrap, 'package-lock.json'));
    const lock = JSON.parse(lockBytes);
    for (const tool of TOOLS.filter(tool => tool !== 'tnpm')) await writeFile(join(projects[tool], 'package-lock.json'), lockBytes);
    return { root, projects, environments, manifest, files, commit: commit.sha,
      manifestSha256: createHash('sha256').update(packageBytes).digest('hex'),
      lockSha256: createHash('sha256').update(lockBytes).digest('hex'), lock,
      close: () => rm(root, { recursive: true, force: true }) };
  } catch (error) { await rm(root, { recursive: true, force: true }); throw error; }
}
