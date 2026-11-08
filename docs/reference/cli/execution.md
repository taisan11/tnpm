# スクリプトと実行

## `run`

```sh
tnpm run
tnpm run build
tnpm run build -- --watch
tnpm run --if-present test
tnpm run ./src/main.ts -- --port 3000
tnpm build
```

- `run` の引数なしは script 一覧（`--json` 対応）。
- スクリプト実行時はローカルの `node_modules/.bin` を PATH に追加し、`pre<SCRIPT>` と `post<SCRIPT>` も扱います。`--ignore-scripts` は前後フックだけを止め、明示した script は動作します。
- `--if-present` はスクリプトが存在しない場合に成功します。
- `.js` / `.jsx` / `.mjs` / `.cjs` / `.ts` / `.tsx` / `.mts` / `.cts` はファイルとして直接実行できます。Node.js 22.15+ 等が必要です。**TypeScript 型検査ではなく変換**です。
- スクリプト名がファイル名と一致する場合はスクリプトが優先されます。組み込みコマンドと同じ名前の script は `tnpm run install` のように実行します。

リモート import（`jsr:`、`npm:`、HTTP(S)）は直接ファイル実行のランナーだけで対応します。任意の `node ...` スクリプトに loader を自動注入するものではありません。[ランナー仕様](../../runner.md)。

## workspaces の `run`

```sh
tnpm run -w app build
tnpm run --workspaces --if-present test
tnpm run --workspaces --parallel build
tnpm run --workspaces --topological --concurrency 4 build
```

| オプション | 効果 |
| --- | --- |
| `-w NAME` / `--workspace NAME` | 対象 workspace を選択（複数回指定可） |
| `--workspaces` | 全 workspace を選択 |
| `--parallel` | 選択した workspace を並列実行 |
| `--concurrency N` | 並列数の上限（正整数。並列実行も有効化） |
| `--topological` | 通常・開発・optional 依存の順序に従い、依存先から実行 |

並列・依存順フラグには workspace の選択が必要です。フラグは script 名の前に指定します。失敗した場合、次の依存段階は開始されません。[workspace 詳細](../../dependency-sources.md#スクリプトの並列依存順実行)。

## `exec` / `x` {#exec}

```sh
tnpm x tsc --version
tnpm exec --package=typescript -- tsc --version
tnpm exec --package=eslint --package=prettier --call "eslint ."
```

まずローカル bin を参照し、なければ一時的な隔離環境へパッケージを取得して実行します。取得先は主に `~/.tnpm/exec/`。`--package` / `-p` は複数指定可、`--call` / `-c` はコマンド文字列を指定します。`--no-cache` で解決し直せます。確認プロンプトはなく `--yes` は互換用フラグです。

## `create`

```sh
tnpm create vite my-app --template react
tnpm create vite@latest my-app
```

`create NAME` は `create-NAME` パッケージを取得し initializer を実行します。`NAME` は必須で、後続の引数を転送します。`tnpm` に効かせたいフラグは `NAME` の前に指定します。`init` へのフォールバックはしません。

## `init`

```sh
tnpm init -y
tnpm init my-app
tnpm --prefix ./examples init my-app
```

ネットワークを使わず指定ディレクトリの `package.json` を作成・補完します。`PATH` はローカルパスです。`--yes` は受理しますが、もともと非対話で既定値を埋めます。

[全オプション](../../cli.md) / [実行と loader](../../runner.md)
