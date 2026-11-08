# 依存関係の操作

`install`、`ci`、`uninstall`、`update`、`outdated`、`prune` は共通の解決器・配置機構を使用します。依存宣言の形式は [package.json](../package-json.md)、配置方法は [.npmrc](../npmrc.md) を参照してください。

## `install` / `i` / `add` {#install}

```sh
tnpm install
tnpm add react
tnpm add typescript -D
tnpm add react@19.0.0 -E
tnpm add jsr:@luca/cases
tnpm install --node-linker=isolated
```

| フラグ | 効果 |
| --- | --- |
| `-D` / `--save-dev` | `devDependencies` に保存 |
| `-O` / `--save-optional` | `optionalDependencies` に保存 |
| `-E` / `--save-exact` | 完全一致バージョンで保存 |
| `--no-save` | manifest / lockfile の保存を抑止 |
| `--package-lock-only` | 配置せず `tnpm-lock.json` まで更新。npm lockfile ではない |
| `--omit=dev,optional,peer` | 指定種別の物理配置を省略 |
| `--production` | dev 依存の配置を省略 |
| `--ignore-scripts` | lifecycle を無効化 |
| `--offline` | キャッシュ外へのネットワーク取得を禁止 |
| `--no-cache` | キャッシュを使わず取得（`--offline` と併用不可） |
| `--node-linker=hoisted\|isolated` | 配置方式を指定 |
| `--store-dir=PATH` | 共有ストアのパス（絶対パス） |
| `--legacy-peer-deps` / `--strict-peer-deps` | peer の解決・不一致処理を制御 |
| `--trust=NAME` | 依存 lifecycle の実行許可を一時的に追加 |
| `--workspace=NAME` / `-w NAME` | workspace の manifest 追加先を選択 |

`install` は `tnpm-lock.json` と `node_modules` を作成します。`npm:`・`jsr:`・`file:`・Git・URL tarball 等の条件付き対応は [依存ソース](../../dependency-sources.md) を参照してください。`--save` / `-S` は互換用に受理しますが動作を追加しません。

## `ci`

```sh
tnpm ci
tnpm ci --ignore-scripts
tnpm ci --node-linker=isolated
```

- `tnpm-lock.json` が必須。manifest・lockfile を照合してから `node_modules` をクリーンに再構築します。
- `package.json` / `tnpm-lock.json` は更新しません。
- パッケージ名の追加指定と `--package-lock-only` は受け付けません。
- `--offline` は必要な内容がストアにある場合だけ成功します。

## `uninstall` / `remove` / `rm` {#uninstall}

```sh
tnpm uninstall react
tnpm rm eslint -w web
```

直接依存の宣言と、その結果不要になった依存・bin を削除します。`-w` で操作対象の workspace を指定できます。`--no-save` や `--package-lock-only` も利用可能です。

## `update`

```sh
tnpm update
tnpm update react
tnpm update --latest
tnpm update astro --latest
tnpm update --latest --save-exact
```

- 通常は manifest に**宣言済みの範囲内**で更新します。宣言が複数 major を許せばその範囲内で major を越えることもあります。
- `--latest` は指定直接依存（省略時は対象全体）の latest へ更新し、`package.json` の宣言も変更します。exact・`^`・`~` や `npm:` / `jsr:` の形式を可能な範囲で維持します。
- 自動更新の実行時 API 互換性までは保証しません。更新後はテストしてください。

## `outdated`

```sh
tnpm outdated
tnpm outdated react --json
tnpm outdated --plain
```

直接依存の `current`（配置版）/ `wanted`（宣言範囲内の候補）/ `latest`（タグの最新版）を表示します。表示対象がなくてもエラーではありません。更新候補があれば終了コード `1`、なければ `0` です。`--plain` は枠・色なし、`--json` は JSON 出力です。

`file:`・Git・workspace 指定など、レジストリ外の依存は更新候補検査から除外します。

## `prune`

```sh
tnpm prune
tnpm prune --omit=dev
tnpm prune --all
```

`node_modules` 内の余剰パッケージを削除します。`--all` / `-a` はディレクトリ全体を削除し、manifest・lockfile・共有キャッシュは保持します。`--all` にパッケージ指定や `--omit` は併用しません。

::: warning isolated の制限
`node-linker=isolated` では通常の `prune` と `rebuild` は未対応です。`tnpm install` による再構築、または `tnpm prune --all` を使用してください。
:::

## グローバル操作

```sh
tnpm install -g typescript
tnpm ls -g
tnpm update -g typescript
tnpm uninstall -g typescript
```

`-g` / `--global` は install・uninstall・update・outdated・ls に対応します。既定は権限昇格不要の `~/.tnpm/global`。workspace 選択と `--no-save` は併用できません。`-g` は共有ストア・`isolated` とは**別の概念**です。

[グローバル配置の仕様](../../dependency-sources.md#グローバルインストール) / [配置と lockfile](../../install-and-lockfile.md)
