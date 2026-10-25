import { TOOLS } from '../lib/fixtures.mjs';

// Argument/validation tests need configuration, not a server or installed tree.
export function caseFixture(live = false) {
  return {
    live,
    registries: Object.fromEntries(TOOLS.map(tool => [tool, live ? 'https://registry.npmjs.org/' : `http://registry.test/${tool}/`])),
    environments: { bun: { BUN_INSTALL_CACHE_DIR: '/fixture/bun/cache' } },
  };
}
