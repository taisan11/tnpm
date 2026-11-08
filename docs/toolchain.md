# TypeScript toolchain (tsrs)

tnpm can replace the **tsc command in package.json scripts** with [tsrs](https://github.com/oxc-project/tsrs) without changing the script text or installed dependencies.

For example:

```json
{
  "scripts": {
    "check": "tsc --noEmit",
    "build": "tsc -p tsconfig.build.json"
  }
}
```

`tnpm run check`, `tnpm check`, and `tnpm run build` continue to invoke the named scripts. During the script and its `pre`/`post` hooks, tnpm puts a temporary `tsc` shim **before** `node_modules/.bin` on `PATH`. The shim runs tsrs, passing arguments and exit status through unchanged.

The repository's release workflow embeds a pinned native tsrs compiler and its adjacent TypeScript standard library files inside the tnpm executable. The files are extracted to the user's cache on first use. Rust builds without `TNPM_TSRS_BUNDLE` use a separately installed `tsrs` executable instead; without either, scripts run the original `tsc`.

## Configuration

Put this in a project or user `.npmrc`:

```ini
toolchain-tsrs=auto
```

| Value | Behavior |
| --- | --- |
| `auto` (default) | Replace `tsc` only if the installed `typescript` is version 7.1.x and tsrs is available |
| `on` | Replace any `tsc` invocation from a script when tsrs is available, regardless of TypeScript version |
| `off` | Never replace `tsc` |

`npm_config_toolchain_tsrs=on` overrides `.npmrc` like other npm-compatible settings. If no tsrs is bundled or installed, `tnpm run` retains normal behavior even with `on`.

tsrs currently follows a pinned TypeScript 7.1 beta and is still experimental. `auto` is intentionally conservative: older TypeScript 5.x/6.x projects retain their compiler. For explicit testing of older projects use `on` and compare diagnostics and emitted files.

## Compatibility and limitations

- `--noEmit`, ordinary builds, and watch/build mode are handled by tsrs.
- tsrs currently does not implement `--init` or `--showConfig`. These command-line operations fall back to the original `tsc` if available.
- Script text is not rewritten. Shell command sequences and nested script calls can use the PATH shim. Explicit paths such as `node node_modules/typescript/bin/tsc` do **not** use the shim.
- Only scripts launched by `tnpm run` and their pre/post hooks are affected. Dependency installation lifecycle hooks, `tnpm exec tsc`, and `import "typescript"` are not rewritten.
- The embedded compiler has an independent version from the project's installed TypeScript. No guarantee of full compatibility is implied.
- Bundling tsrs increases the size of official tnpm binaries. Runtime extraction uses the local user cache; no network requests are made.

The embedded tsrs compiler and TypeScript standard library definitions are licensed under Apache-2.0, copyright their respective contributors and Microsoft Corporation. See [tsrs LICENSE and NOTICE](https://github.com/oxc-project/tsrs) for attribution. The release bundle also carries the available upstream notice and license files.

## Maintainer builds

To embed your own compiler build, set `TNPM_TSRS_BUNDLE` to an absolute path to a `tar.gz` archive containing a platform-native `tsrs` (or `tsrs.exe`) and `lib.d.ts` at its root. The release workflow creates that archive with `.github/scripts/bundle_tsrs.py` and passes the location to Cargo. Ordinary `cargo build` builds are small and do not fetch anything.
