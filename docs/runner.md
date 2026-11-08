# Runner

`tnpm run` executes a named `package.json` script or a JavaScript/TypeScript entry file. A script name always takes priority if it matches a file name.

```sh
tnpm run build -- --watch
tnpm run ./src/main.ts -- --port 3000
tnpm run ./src/main.js
```

## Direct file execution

Install Node.js 22.15 or later (or 23.5 or later for Node 23). Oxc's Rust transformer and resolver and SWC's CommonJS module transform are built into tnpm; no npm loader package is required. tnpm starts Node with its own synchronous module hooks, invokes its bridge for transformations and unresolved imports, and leaves execution to Node. TypeScript is transformed, not type-checked. Run a separate type checker when needed.

The entry must end in `.js`, `.jsx`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.mts` or `.cts`. Node's normal module resolution is attempted first; Oxc resolves imports that Node cannot find, including supported `tsconfig.json` path aliases and TypeScript extensions. Oxc's default transform options apply, with SWC lowering import/export syntax for CommonJS files; tsconfig transform options and source maps are not yet supported. Source files requiring transformation invoke a separate tnpm process per module, so this is not yet optimized for large graphs. Hooks are installed in the main Node process; worker threads and independently spawned Node processes do not automatically inherit them.

Entry paths are relative to the invocation directory, or to `--prefix` when specified. Direct execution does not require a package.json. Arguments after the entry are passed directly to Node without shell interpretation. The child inherits standard input, output and error; its normal exit code is propagated. Entry execution does not run npm lifecycle hooks. Node flags before the entry and watch mode are not currently supported; use Node directly for those.

## JSR, npm and URL imports

Direct execution also resolves `jsr:@scope/package@range[/export]`, `npm:package@range[/export]` and HTTP(S) imports. JSR uses its official npm compatibility distribution; matching local installations are preferred, otherwise packages are installed in isolated runner caches with lifecycle hooks disabled. HTTP modules retain their final URL, resolve relative and dynamic imports, and transform TypeScript. JSON imports require `with { type: 'json' }`. Node's existing file, builtin and data URL handling is retained.

Use `tnpm run --offline FILE` for cache-only execution or `tnpm run --no-cache FILE` to reload remote modules. URL caches are not pinned in the project lockfile. Remote CommonJS, URL entry points, Deno APIs and import maps are not supported. These features do not sandbox imported code. See [JSR and URL imports](jsr-and-url-imports.md) for cache, authentication, protocol boundaries and limitations.

## Package scripts

With no operand, `tnpm run` lists scripts (`--json` emits JSON). Named scripts run from the project root using the system shell, with local `node_modules/.bin` directories on PATH. Arguments are appended to the main script, not its hooks. `pre<name>` and `post<name>` run before and after a successful main script respectively, unless `--ignore-scripts` is set. `--if-present` suppresses errors for missing script names. For direct file operands, a missing file is still an error.

Lifecycle environment includes `npm_lifecycle_event`, `npm_lifecycle_script`, `npm_package_json`, `npm_package_name`, `npm_package_version`, `npm_execpath` and `INIT_CWD`. Scripts execute as written; tnpm does not inject the runner into arbitrary shell commands.
