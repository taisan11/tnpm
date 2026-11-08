# 認証と公開

npm 互換レジストリとの認証、ローカルディレクトリの梱包と公開を扱います。**provenance / OIDC trusted publishing や unpublish などは未対応**です。詳細は [認証と公開の仕様](../../publishing.md)。

## `login` / `adduser` {#login}

```sh
tnpm login
tnpm login --registry https://registry.example.com/ --scope @team
tnpm login --auth-type=legacy
tnpm adduser --auth-type=legacy
```

既定はブラウザ URL を表示する Web 認証で、自動起動はしません。`--auth-type=legacy` はユーザー名・パスワードを対話的に入力する方式です。成功した場合、選択したレジストリにスコープされたトークンを **user .npmrc** へ保存します。プロジェクト .npmrc は書き換えません。

## `logout` / `whoami` {#logout}

```sh
tnpm whoami
tnpm whoami --json
tnpm logout
```

`whoami` は認証中のユーザー名を表示し、`logout` は選択された認証情報を失効・削除します。別の設定ファイルや環境変数に残る資格情報まで一括削除するものではありません。

## `pack`

```sh
tnpm pack
tnpm pack ./packages/widget --pack-destination ./artifacts
tnpm pack --dry-run --json
```

ローカルディレクトリを npm 形式の tarball にまとめます。`--dry-run` は保存しません。`files` / `.npmignore` / `.gitignore`、強制除外対象や lifecycle の挙動は [詳しい pack 仕様](../../publishing.md#pack--publish) を確認してください。`--dry-run` でも前処理フックは実行されるため、副作用を避けたい場合は `--ignore-scripts` も指定します。

## `publish`

```sh
tnpm publish --dry-run --ignore-scripts
tnpm publish --access=public
tnpm publish ./packages/widget --tag=next --otp=123456
```

レジストリへ公開します。`name` / `version` が必須で、`private: true` は拒否。`package.json` の `publishConfig` は文字列の `registry` / `tag` / `access` のみです。CLI の指定が `publishConfig` より優先します。`publish` は tarball をローカルに保存しません。

公開時の機密ファイル確認やアカウント側のレジストリ制約は利用者の責任で確認してください。

## `dist-tag`

```sh
tnpm dist-tag ls @team/widget
tnpm dist-tag add @team/widget@1.2.3 next
tnpm dist-tag rm @team/widget next
```

公開済みパッケージの dist-tag を表示・追加・削除します。`ls` / `list`、`add`、`rm` / `remove` に対応。変更操作には認証が必要です。`--dry-run` は未対応です。

## 共通フラグ

| フラグ | 用途 |
| --- | --- |
| `--registry URL` | 対象レジストリ |
| `--auth-type web\|legacy` | `login` / `adduser` の認証方法 |
| `--scope @team` | 認証に使うスコープ |
| `--otp CODE` | OTP ヘッダー |
| `--tag TAG` | 公開時・タグ追加時のタグ |
| `--access public\|restricted` | 公開のアクセス区分 |
| `--dry-run` | `pack` / `publish` の送信・保存抑止（フックは別） |
| `--pack-destination PATH` | `pack` の tarball 保存先 |

[詳細・対応外の操作](../../publishing.md) / [.npmrc](../npmrc.md)
