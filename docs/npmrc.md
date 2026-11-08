# .npmrc と npm: エイリアス

`login` / `logout` による user npmrc の更新と公開時の `publishConfig` は [認証と公開](publishing.md) を参照してください。

[npm v12 の .npmrc 仕様](https://docs.npmjs.com/cli/v12/configuring-npm/npmrc)を参考に、tnpm で使用する設定を読み込みます。npm の全コマンド・全設定オプションの互換実装ではありません。

## 設定ファイルと優先順位

低い順に次の設定を重ねます。

1. tnpm 内蔵の既定値
2. `$PREFIX/etc/npmrc`（`--globalconfig` / `npm_config_globalconfig` で変更可能）
3. `$HOME/.npmrc`（`--userconfig` / `npm_config_userconfig` で変更可能）
4. プロジェクトルートの `.npmrc`
5. `npm_config_*` 環境変数（接頭辞の大文字・小文字を区別しません）
6. 対応する CLI オプション

ユーザー設定の `prefix` / `globalconfig` からグローバル設定の場所を選択することもできます。npm 本体のインストール先にある builtin npmrc は読みません。`-g` の既定 prefix は管理者権限不要の `~/.tnpm/global` で、`--prefix` / `npm_config_prefix` / npmrc の `prefix` で変更できます。[グローバル配置](dependency-sources.md#グローバルインストール)。

### 書式

- `key = value`、値を省略した boolean フラグ、単一／二重引用符付き文字列。
- `#` / `;` で始まるコメントと、引用符なしの値の末尾コメント。値の `#` / `;` は引用符で囲むか `\#` / `\;` として記述します。
- `${NAME}` は環境変数で展開し、未定義ならそのまま保持します。`${NAME?}` は未定義なら空文字列です。
- `key[] = value` を繰り返す配列。優先度の高いファイルの配列で低いファイルの配列を置き換えます。`get --json` では配列として表示し、`omit[]` は実際の配置にも適用します。
- `cache` / `prefix` / `store-dir` / 設定ファイル・証明書ファイルのパスでは `~/` をホームディレクトリに展開します。

```ini
omit[] = dev
omit[] = optional
node-options = "${NODE_OPTIONS?} --use-system-ca"
```

未知のキーは保持して `get` で参照できますが、npm の全設定を動作に反映するわけではありません。npm v12 の未知キー警告や `npm config set/edit` は未実装です。上記 `node-options` の例は読み込み・環境変数展開の例で、スクリプト実行への適用は未対応です。

## 分離インストールと共有ストア

ユーザー共通の `$HOME/.npmrc` に設定すると、全プロジェクトの既定値を変更できます。プロジェクトの `.npmrc` や CLI はこれより優先されます。

```ini
node-linker=isolated
store-dir=~/.tnpm/cache
```

- `node-linker=hoisted`（既定）: 従来どおり依存を巻き上げて配置。
- `node-linker=isolated`: `node_modules/.tnpm/` の仮想ストアに各実体を置き、宣言された依存だけをディレクトリリンクで公開。`tnpm ci` と通常の `install` に適用。
- `store-dir`: 全プロジェクトで共有するパッケージストアの親ディレクトリ。既定は `~/.tnpm/cache`。取得用の `packages-v2/` と、分離インストール時にファイル単位のハードリンクを作成する `files-v1/` がその下に作られます。絶対パスのみ受け付けます。ハードリンクを使うにはプロジェクトと同じディスク／ボリュームに置いてください。
- `npm_config_node_linker` / `npm_config_store_dir` 環境変数でも変更できます。CLI の `--node-linker` / `--store-dir` が最優先です。

詳しい配置方式と現在の制限は [インストールと lockfile](install-and-lockfile.md) を参照してください。

## レジストリと認証

`@scope:registry` はそのスコープの参照先に適用します。明示的な `--registry` はスコープ設定より優先します。`view` とインストール系コマンド、`audit` は同じ選択・認証処理を使います。`audit-level`（info / low / moderate / high / critical / none）で audit の終了コードの閾値を設定でき、CLI の `--audit-level` が優先します。

```ini
@jsr:registry = https://npm.jsr.io/
@myorg:registry = https://registry.example.com/team/
//registry.example.com/team/:_authToken = ${NPM_TOKEN}
```

認証はホスト・ポート・パスに限定し、最も具体的なスコープを使います。

- `:_authToken`: Bearer token
- `:_auth`: base64 の Basic 認証文字列
- `:username` + `:_password`: ユーザー名と base64 のパスワード
- `:certfile` + `:keyfile`: PEM 形式のクライアント証明書と秘密鍵
- `:cafile`（または共通の `cafile`）: PEM 形式の CA 証明書

スコープのない認証・クライアント秘密鍵設定はエラーです。未展開の認証用環境変数も送信前にエラーにします。認証を付けたリダイレクトが元の origin / 許可パスから外れる場合は追従しません。認証付きの取得結果を共有キャッシュに保存せず、`get` では認証情報を隠します。tarball の認証はその URL のスコープで判定し、別のホストやパスにレジストリの認証を転送しません。

## JSR の npm エイリアス

`jsr:@scope/package@RANGE` を直接使用できます。`tnpm add jsr:@luca/cases` は `jsr:` 指定を保存し、内部で公式の npm 互換パッケージを解決します。`@jsr:registry` の既定値は `https://npm.jsr.io/` なので設定追加は不要です。JSR は npm の `--registry` 指定とは独立してこのレジストリを使用します。以下のエイリアス形式も引き続き使えます。[JSR と URL import](jsr-and-url-imports.md)。

たとえば `package.json` に次の依存がある場合:

```json
{
  "dependencies": {
    "@taisan11/vite-plugin-budoux-build": "npm:@jsr/taisan11__vite-plugin-budoux-build@^1.0.1"
  }
}
```

組み込みの `@jsr:registry=https://npm.jsr.io/`（npmrc / `--registry` で上書き可）を使用して `@jsr/taisan11__vite-plugin-budoux-build` を取得し、`node_modules/@taisan11/vite-plugin-budoux-build` に配置します。エイリアス名をレジストリには問い合わせません。

`outdated` はエイリアス名を表・JSON のキーとして保ち、参照先の declared range から wanted、参照先の `latest` タグから latest を調べます。current はエイリアス名のインストール先から読み、存在しなければ `MISSING` です。`MISSING` 自体は `.npmrc` のエラーではありません。

current が latest と一致する依存は、wanted の値に関係なく表・JSON から除外します。表示対象がなければ `All dependencies are up to date.`（表のヘッダーはなし）と表示し、JSON 出力は `{}`、終了コードは 0 です。

`install` / `update` / `ci` / `ls` でも参照先の名前・バージョンを検証します。同じバージョンでも実体の異なるエイリアスは共有せず、必要ならネストします。入れ子の `npm:`、file/Git/URL/workspace 形式の参照先は未対応です。

## テスト

CLI 結合テストはローカル HTTP レジストリだけを使用します。設定優先順位、配列・環境変数展開、エイリアスとスコープ別ルーティング、認証ヘッダー・秘密情報の非表示、パス外リダイレクト拒否、表示幅による表の整列を検証します。実際の外部レジストリや TLS クライアント証明書での接続試験は行いません。
