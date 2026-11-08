# CLI コマンド一覧

`tnpm [OPTIONS] <COMMAND|SCRIPT> [ARGS...]`

tnpm は組み込みコマンドを優先して解釈します。最初の名前が組み込みコマンドと一致しない場合は `package.json` のスクリプトとして実行します（例: `tnpm build`）。衝突時は `tnpm run install` のように明示します。

## コマンド索引

| コマンド | 別名 | 説明 |
| --- | --- | --- |
| [`install`](./dependencies.md#install) | `i`、`add` | 依存の解決、追加、配置 |
| [`ci`](./dependencies.md#ci) | — | lockfile 固定のクリーンインストール |
| [`uninstall`](./dependencies.md#uninstall) | `remove`、`rm` | 依存の削除 |
| [`update`](./dependencies.md#update) | — | 依存の更新 |
| [`outdated`](./dependencies.md#outdated) | — | 更新可能な依存を検出 |
| [`prune`](./dependencies.md#prune) | — | 余剰依存を削除 |
| [`run`](./execution.md#run) | スクリプト名 | スクリプトまたは JS / TS ファイルを実行 |
| [`exec`](./execution.md#exec) | `x` | ローカル bin / 一時取得した bin の実行 |
| [`create`](./execution.md#create) | — | `create-NAME` initializer を実行 |
| [`init`](./execution.md#init) | — | `package.json` を初期化 |
| [`ls`](./inspection.md#ls) | — | インストール済み依存を列挙 |
| [`view`](./inspection.md#view) | — | レジストリのメタデータ |
| [`why` / `explain`](./inspection.md#explain) | `why` | 依存の要求元と経路 |
| [`audit`](./inspection.md#audit) | — | 脆弱性 advisory の照会 |
| [`fund`](./inspection.md#fund) | — | funding URL の列挙 |
| [`login` / `adduser`](./publishing.md#login) | — | レジストリ認証 |
| [`logout` / `whoami`](./publishing.md#logout) | — | 認証解除・ユーザー確認 |
| [`pack`](./publishing.md#pack) | — | tarball 作成 |
| [`publish`](./publishing.md#publish) | — | レジストリ公開 |
| [`dist-tag`](./publishing.md#dist-tag) | — | タグ操作 |
| [`get`](./maintenance.md#get) | — | npm 設定の確認 |
| [`cache`](./maintenance.md#cache) | — | キャッシュ容量・削除 |
| [`version`](./maintenance.md#version) | — | プロジェクトのバージョン更新 |
| [`upgrade`](./maintenance.md#upgrade) | — | tnpm 本体更新 |
| [`alias`](./maintenance.md#alias) | — | npm コマンドの代替 alias |
| [`rebuild`](./maintenance.md#rebuild) | — | bin / 許可済み lifecycle の再実行 |

## 引数の位置に注意

`run`、`exec`、`create` では対象名以降のフラグは**子プロセスに転送**されます。tnpm 自身のフラグは対象より前に置きます。

```sh
tnpm --prefix ./app run build -- --watch
tnpm exec --package=typescript -- tsc --version
tnpm create vite my-app --template react
tnpm run --workspaces --parallel build
```

`--all`、`--depth`、`--latest` など一部のコマンド固有オプションはコマンドより**後**に指定してください。`--json` の有効範囲なども含めて [全オプション](../../cli.md) に記載しています。

## 終了コード

通常は成功 `0`、失敗 `1` です。`outdated` は更新候補があると `1` を返します。`run` などの子プロセスの終了コードは可能な範囲で引き継ぎます。通常結果は stdout、警告・進捗・ビルド情報は stderr です。

[CLI 出力と進捗](../../cli-output.md) / [対応範囲と制限](../../compatibility.md)
