# 認証とパッケージ公開

tnpm は npm 互換レジストリに対する `login` / `adduser` / `logout` / `whoami`、ローカルディレクトリの `pack` / `publish`、`dist-tag` に対応します。npm の完全互換実装ではありません。

## ログイン

```sh
tnpm login
tnpm whoami
tnpm login --registry https://registry.example.com/npm/ --scope @team
tnpm login --auth-type legacy --otp 123456
tnpm adduser --auth-type legacy
tnpm logout
```

- 既定は Web ログインです。表示された URL を自分でブラウザに開いて認証してください。ブラウザの自動起動は行わず、最大約5分間完了を待ちます。`auth-type=legacy` を npmrc / `npm_config_auth_type` で指定することもできます。
- Web API が HTTP 404 / 405 / 500 / 501 を返した場合は legacy にフォールバックします。legacy はユーザー名・パスワードをプロンプトで読みます。対話端末ではパスワードを非表示にし、パイプでは stdin の1行ずつを読みます。パスワードを CLI 引数や npmrc に保存する機能はありません。
- `adduser --auth-type legacy` はメールアドレスも読み、レジストリにアカウント作成を要求します。実際に作成できるかはレジストリ次第です。Web モードは `login` と同じ認証 API を利用します。
- 成功時のみ **user npmrc**（既定 `~/.npmrc`、`--userconfig` / `npm_config_userconfig` で変更可能）に `//host/path/:_authToken` を保存します。同じ認証スコープの古い Basic 認証設定を除去し、他のキー・コメントは保持します。project npmrc は変更しません。
- `--scope @team` は対象スコープのレジストリを選択し、ログイン時に `@team:registry` も user npmrc に保存します。`--registry` はスコープ設定より優先します。
- user npmrc は一時ファイル経由で置き換え、Unix では mode 0600 にします。user npmrc 自体が symlink の場合は拒否します。Windows のアクセス制御は保存先ディレクトリから継承します。
- `whoami` はトークンの場合 `/-/whoami`、Basic 認証の場合ローカルのユーザー名を使用します。`--json` はユーザー名の JSON 文字列です。
- `logout` は選択されたトークンをレジストリで失効させてから、その認証スコープの user npmrc エントリーを除去します。失効に失敗したらファイルを変更しません。Basic 認証はローカル設定のみ除去します。project npmrc・環境変数・global npmrc にも同じ資格情報があれば、それらは残ります。スコープとレジストリの関連付けも残します。

認証情報はレジストリのホスト・ポート・パスに限定します。ログイン・公開・タグ操作はリダイレクトを追従せず、Web ログインの完了確認 URL も同じ origin / レジストリパス内に限定します。プロキシが別 origin の完了 URL を返す形式は未対応です。実際のレジストリには HTTPS を使用してください。

## pack / publish

証明書だけを使用した認証の `logout` は未対応です。TLS identity の設定を手動で取り除いてください。

```sh
tnpm pack --dry-run --json              # 内容確認のみ（tarball を保存しない）
tnpm pack --pack-destination ./artifacts
tnpm publish --dry-run --ignore-scripts # 送信せず、フックも実行せず確認
tnpm publish --access public
tnpm publish ./packages/widget --tag next --otp 123456
tnpm --prefix ./packages/widget publish
```

- 対象はローカルディレクトリのみ（省略時はプロジェクトルート）。`package.json` の名前・バージョンが必須です。名前は小文字の npm 名、バージョンは `1.2.3` / `1.2.3-beta.1` など正規形の SemVer で指定してください。`private` パッケージの publish は `--dry-run` でも拒否します（pack は可能）。
- `pack` は実行時の作業ディレクトリ、または `--pack-destination` に `<name>-<version>.tgz` を保存します。スコープ付きは `team-widget-1.2.3.tgz` の形式です。同名ファイルは一時ファイルから置き換えます。`publish` は tarball をディスクに保存しません。
- `files` 配列で含めるパス・glob を指定できます。ルートの `.npmignore`（なければ `.gitignore`）を使用し、`files` があればルートの ignore ファイルより配列を優先します。サブディレクトリの ignore ファイル、`!` による再包含も扱います。
- `package.json`、ルートの README / LICENSE / LICENCE / COPYING（拡張子付き可）、`main`、`bin` は原則として含めます。ただし、下記の強制除外・symlink の除外は優先されます。
- **強制除外**: `node_modules`、`.git` / `.hg` / `.svn` / `.tnpm`、`.npmrc`、ignore ファイル、`.env` / `.env.*`、`*.pem` / `*.key`、`id_rsa` / `id_ed25519`、既存 `*.tgz`、主要 lockfile（`npm-shrinkwrap.json` も含む）、`.DS_Store`、`npm-debug.log`。symlink は参照先を辿らず除外します。これは npm より保守的な仕様です。
- ファイル名をソートし、tar / gzip の時刻を固定して tarball を作ります。通常ファイルは mode 0644、実行可能ファイルと `bin` は 0755。SHA-512 integrity と SHA-1 shasum を計算してレジストリの公開メタデータに含めます。
- `publishConfig` は文字列の `registry` / `tag` / `access` のみ対応し、その他は黙って無視せず拒否します。優先順位は CLI > publishConfig > 通常のレジストリ / npmrc 設定です。`access` は `public` / `restricted`、後者はスコープ付きのみ。tag は既定 `latest` で、バージョン・バージョン範囲は指定できません。
- 公開にはレジストリ別のトークン・Basic 認証または client TLS identity が必要です。`--otp CODE` / npmrc の `otp` で OTP ヘッダーを送れます。レジストリが OTP を要求して失敗した場合は、コードを付けて再実行してください。ブラウザ MFA の追加チャレンジへの自動対応はありません。
- `pack --json` は1要素の配列、`publish --json` はオブジェクトで、`id` / `filename` / `size` / `unpackedSize` / `shasum` / `integrity` / `files` / `dryRun` を返します。`files` はパス・サイズ・mode。人間向け出力でもファイル一覧を stderr に表示します。

### lifecycle

設定済みの user / global npmrc、レジストリの keyfile / certfile は、独自の名前でパッケージ内に置かれている場合も梱包から除外します。

pack は `prepack` → `prepare` → tarball 作成 → `postpack`。publish はその前に `prepublishOnly`、送信成功後に `publish` → `postpublish` を実行します。prepare までのフックが変更した manifest は再読み込みします。`prepublish` は実行しません。

`--ignore-scripts` / npmrc の `ignore-scripts=true` ですべて無効化します。**`--dry-run` だけでは前処理フックを止めません**（送信後の publish / postpublish は実行しません）。副作用を避けるなら `--dry-run --ignore-scripts` を併用してください。`--json` 時のフック標準出力は stderr に流します。フック失敗時の変更は巻き戻しません。公開成功後に後処理フックが失敗した場合も、公開自体は取り消されません。

## dist-tag

```sh
tnpm dist-tag ls @team/widget --json
tnpm dist-tag add @team/widget@1.2.3 next
tnpm dist-tag rm @team/widget next
```

`ls` / `list` は省略可能で、パッケージ名省略時はローカル manifest の名前を使用します。`add` は正規形の明示バージョンが必須、タグ省略時は `--tag` / 設定の `tag` / `latest`。`rm` / `remove` はタグ必須です。変更には認証が必要です。レジストリとスコープ別認証を尊重し、キャッシュを使用しません。`--dry-run` は未対応として拒否します。

## 今回の対応外

`unpublish`、`deprecate`、`owner` / `access` / `token` / `profile` 管理、provenance / OIDC trusted publishing、既存 tarball / Git / レジストリ指定からの pack / publish、bundled dependencies の梱包、`workspace:` 範囲の書き換え、workspace 一括公開は未対応です。個別ディレクトリは `--prefix` を使って指定してください。認証・公開・タグ操作の通信は `--offline` で拒否しますが、`pack` と `publish --dry-run` は通信せず動作します。

強制除外だけで任意の秘密情報の検出はできません。特に manifest や独自ファイルに秘密がないことを、公開前に確認してください。
