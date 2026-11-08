# `.npmrc`

tnpm は npm 形式のフラットな INI 設定を読み、実装が使用するキーを解釈します。npm の設定すべてを実装しているわけではありません。構文・認証境界の詳細は [既存の .npmrc 仕様書](../npmrc.md) を参照してください。

## どのファイルを読むか

**後の設定ほど優先**されます。

1. tnpm 内蔵の既定値
2. global npmrc（通常は `<prefix>/etc/npmrc`）
3. user npmrc（通常は `~/.npmrc`）
4. プロジェクトルートの `.npmrc`
5. `npm_config_*` 環境変数
6. 対応する CLI オプション

`--userconfig` / `npm_config_userconfig` で user 設定のパスを、`--globalconfig` / `npm_config_globalconfig` で global 設定のパスを指定できます。`~/.npmrc` の `prefix` / `globalconfig` も global 設定の読み込み元に影響します。

## 主なキー一覧

| キー | 既定値 | 利用箇所・補足 |
| --- | --- | --- |
| `registry` | `https://registry.npmjs.org/` | npm レジストリの URL |
| `@jsr:registry` | `https://npm.jsr.io/` | JSR の npm 互換レジストリ |
| `@scope:registry` | 未設定 | スコープごとのレジストリ。`--registry` を明示した場合は CLI が優先 |
| `node-linker` | `hoisted` | `hoisted` または `isolated`。CLI: `--node-linker` |
| `store-dir` | `~/.tnpm/cache` | 共通ストアの親ディレクトリ。**展開後に絶対パス必須**。CLI: `--store-dir` |
| `cache` | `~/.npm` | npm 互換設定として読み込み・表示。tnpm の主なキャッシュの保存先を変更するキーではない |
| `prefix` | 通常 `/usr/local`。`-g` の既定は `~/.tnpm/global` | グローバル配置や設定ファイル参照で使用 |
| `ignore-scripts` | `false` | lifecycle スクリプトを無効化 |
| `engine-strict` | 未設定（false 相当） | engines の不一致をエラー化 |
| `legacy-peer-deps` | 未設定（false 相当） | peer 自動解決を無効化 |
| `strict-peer-deps` | 未設定（false 相当） | peer の不一致をエラー化 |
| `omit` / `omit[]` | 未設定 | 物理配置を省略する `dev` / `optional` / `peer` |
| `audit-level` | `info` 相当 | `audit` の失敗閾値 |
| `tag` | `latest` | 公開・タグ操作等の既定タグ |
| `auth-type` | `web` 相当 | `login` の `web` / `legacy` |
| `otp` | 未設定 | 公開・タグ操作の OTP コード |
| `git-tag-version` | `true` | `version` の Git コミット・タグ作成 |
| `tag-version-prefix` | `v` | `version` で Git タグに付ける接頭辞 |
| `allow-same-version` | `false` | `version` で同じバージョンを許可 |
| `preid` | 空文字列 | `version` の prerelease 識別子 |

上記は主なキーです。`cafile`・`certfile`・`keyfile` やレジストリ別の認証キーも使用します（後述）。未知の設定は `get` で参照できても実際の機能に影響しない場合があります。

## 推奨例: 共有ストア＋分離インストール

全プロジェクトの既定値を変更するには、ユーザーの `~/.npmrc` に次を追加します。

```ini
node-linker=isolated
store-dir=~/.tnpm/cache
```

`isolated` は `node_modules/.tnpm/` にパッケージ実体を置き、ファイル単位のハードリンクで共有します。Windows の依存ディレクトリは Junction、Linux/macOS は symlink です。**ストアとプロジェクトが別ボリュームの場合は失敗し、コピーへのフォールバックはありません。** 直接ファイルを編集すると他のインストールにも影響し得ます。

詳しくは [配置方式](../install-and-lockfile.md#配置方式と共有ストア) と [ストア設計](/architecture/storage) を参照してください。

## レジストリ・認証

```ini
@team:registry=https://registry.example.com/npm/
//registry.example.com/npm/:_authToken=YOUR_TOKEN
```

認証情報は **`//host/path/:key`** のようにレジストリスコープを付けて設定してください。スコープなしの `_authToken`・`_auth`・`_password`・`username`・`certfile`・`keyfile` はエラーです。実運用ではトークンをリポジトリにコミットせず、`${NPM_TOKEN}` などの環境変数参照を使ってください。

| キー | 用途 |
| --- | --- |
| `//host/path/:_authToken` | Bearer token |
| `//host/path/:_auth` | base64 で符号化した Basic 認証 |
| `//host/path/:username` と `:_password` | ユーザー名と base64 パスワード |
| `//host/path/:certfile` / `:keyfile` | クライアント TLS 証明書 / 秘密鍵 |
| `cafile` または `//host/path/:cafile` | CA 証明書 |

認証スコープ外へのリダイレクトで秘密情報を転送せず、`tnpm get` は認証設定を表示しません。

## INI 書式

```ini
# 行コメント
ignore-scripts=false
omit[]=dev
omit[]=optional
store-dir="~/tnpm-store"
```

- `key=value`、`key` 単独（`true` と解釈）、`key[]=value` の繰り返しに対応。
- コメントは `#` と `;`。引用符のない値の後方コメントも認識します。
- `${ENV_NAME}` を環境変数で置換。未定義の場合は元の文字列を維持し、`${ENV_NAME?}` は空にします。
- `cache`、`prefix`、`store-dir` 等のパスでは `~/` をホームディレクトリに展開します。
- `npm_config_node_linker` / `npm_config_store_dir` のような環境変数ではアンダースコアをハイフンに正規化します。
- `tnpm get registry` または `tnpm get --json` で有効な設定を確認できます。ただし認証関連キーは秘匿されます。

## 制約

`npm config set` / `npm config edit` は未実装です。`get` が未知のキーを表示できることは、その設定がコマンドに反映される保証にはなりません。詳細な設定解釈は [npmrc 実装仕様](../npmrc.md) を参照してください。
