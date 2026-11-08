# 管理・保守

## `get`

```sh
tnpm get registry
tnpm get node-linker store-dir
tnpm get --json
```

読み込んだ npm 設定を表示します。`package.json` を取得するコマンドではありません。認証トークン、パスワード、秘密鍵設定などは表示を拒否します。詳細: [.npmrc](../npmrc.md)。

## `cache`

```sh
tnpm cache
tnpm cache --json
tnpm cache rm
```

`cache` は tnpm のキャッシュ種類ごとのサイズを表示します。`cache rm` は**確認なし**で `~/.tnpm/cache` と `~/.tnpm/exec` の管理キャッシュを削除します。npm のキャッシュやプロジェクト `node_modules`、`tnpm-lock.json` は削除しません。

::: warning 独自 store-dir
`store-dir` を標準の `~/.tnpm/cache` 以外に設定した場合、`tnpm cache rm` はその追加ディレクトリを削除しません。インストールや exec の実行中にキャッシュ削除しないでください。
:::

## `upgrade`

```sh
tnpm upgrade
tnpm upgrade --json
```

GitHub Releases の最新安定版を取得し、提供された digest で検証して自身を置き換えます。追加引数は受け付けません。実行ファイルを更新できる権限が必要です。

## `alias`

```sh
tnpm alias create
tnpm alias delete
```

`alias create` は利用環境の `npm` 入力を tnpm に向けるためのシェル設定等を追加します。Unix は bash / zsh / fish の対話シェル向け alias。Windows は専用の `npm.cmd` / `npm.ps1` shim を作成します。元の npm をアンインストールする操作ではありません。完全な npm 互換性もありません。

## `version`

```sh
tnpm version
tnpm version patch
tnpm version prerelease --preid=beta
tnpm version minor --no-git-tag-version
```

現在のバージョンを表示するか、manifest と既存の lockfile のルート版を更新します。`major` / `minor` / `patch` / `premajor` / `preminor` / `prepatch` / `prerelease` / `from-git` / 明示 SemVer に対応します。Git 管理下では原則としてクリーン状態を確認してコミット・タグを作成します。`--no-git-tag-version` で無効化できます。

`tnpm --version` は**本体バージョンの表示**であり、`tnpm version` とは別です。

## `rebuild`

```sh
tnpm rebuild
tnpm rebuild esbuild
tnpm rebuild my-native --trust my-native
```

インストール済み依存を再解決せず、bin を生成し直し、許可された依存の lifecycle を実行します。`package.json` と `tnpm-lock.json` は変更しません。ネットワーク取得はありません。`--ignore-scripts` では bin 再生成だけです。現時点で `isolated` 配置は未対応です。

[詳細な信頼設定](../../security-and-diagnostics.md) / [CLI オプション](../../cli.md)
