# JSR と URL import

## JSR パッケージのインストール

```sh
tnpm add jsr:@luca/cases
tnpm add 'jsr:@luca/cases@^1'
tnpm add 'cases@jsr:@luca/cases@1' -E
tnpm view jsr:@luca/cases version
```

[JSR](https://jsr.io/docs/introduction) の `jsr:@scope/package@RANGE` 指定を受け付けます。JSR の公式 [npm 互換レジストリ](https://jsr.io/docs/npm-compatibility) を利用し、たとえば `@luca/cases` は内部で `@jsr/luca__cases` を取得して `node_modules/@luca/cases` に配置します。

```json
{
  "dependencies": {
    "@luca/cases": "jsr:@luca/cases@^1.0.0"
  }
}
```

この形式を manifest に直接書くこともできます。元の `jsr:` 指定を保持し、lockfile に npm 互換パッケージの実名・解決済みバージョン・tarball・integrity を保存します。通常・推移依存・peer、`ci`、`ls`、`outdated`、`update` / `update --latest` も共通の解決器を使います。追加時に版を省略した場合は latest を選び、既定では `^VERSION`、`-E` では exact を保存します。`update --latest` は `jsr:` と exact / `^` / `~` の形式を維持します。

`@jsr:registry` の既定値は `https://npm.jsr.io/` なので npmrc の追記は不要です。ミラーを使う場合は `.npmrc` の `@jsr:registry`、全体を一時的に変更する場合は `--registry` で上書きできます。既存の `npm:@jsr/scope__name@RANGE` 形式も使えます。認証、TLS、キャッシュ、tarball integrity、スクリプト許可制は通常のレジストリ依存と同じです。

インストール指定には export のサブパスを付けません。インストール後は `import ... from '@luca/cases'` や公開されたサブパスを使用します。

## インストールなしの jsr: / npm: import

```ts
import { camelCase } from 'jsr:@luca/cases@1';
import { something } from 'jsr:@scope/package@^1/feature';
import value from 'npm:some-package@1';

console.log(camelCase('hello world'));
```

```sh
tnpm run ./main.ts
tnpm run --offline ./main.ts
```

直接ファイル実行のランナーで `jsr:` / `npm:` import を解決します。名前・実体・バージョンの合うローカル依存があれば再利用し、なければ `~/.tnpm/exec/runner-*` に隔離して取得します。project の manifest / lockfile / node_modules は変更しません。パッケージの exports と Node の import / require 条件を尊重し、推移依存にも共通インストーラーを使います。

自動取得時の install / prepare / 暗黙の node-gyp は**常に停止**します。インストールフックが必要なパッケージは明示的に add / rebuild してから実行してください。`jsr:` import は Deno のネイティブ registry loader ではなく npm 互換配布物を Node で動かす方式です。JSR パッケージ自身の Node 対応が必要です。

各隔離環境の lockfile は解決結果を固定します。通常の実行では再利用し、`--offline` では足りないキャッシュを通信で補いません。`--no-cache` は隔離環境を再解決します（既存のローカル依存は引き続き優先します）。`tnpm cache rm` で隔離環境も削除できます。

## HTTP(S) import

```ts
import { value } from 'https://example.com/lib/value.ts';
const { extra } = await import('https://example.com/lib/extra.js');
```

HTTP(S) で配信された ES module を取得し、JavaScript / TypeScript を既存の Oxc で変換して Node に渡します。リダイレクト後の URL を基準に `./`、`../`、`/`、`//host/` を解決し、再 export・動的 import・query / fragment を扱います。`import.meta.url` は最終的な配信 URL です。

- `.js` / `.mjs` / `.ts` / `.mts` / `.tsx` / `.jsx`、または JavaScript / TypeScript の Content-Type に対応します。拡張子なしの CDN URL は Content-Type で判定します。HTML、未対応形式、不正な UTF-8、構文エラー、16 MiB を超えるソースは拒否します。
- JSON は `with { type: 'json' }` を明示して import できます。
- URL モジュールからのパッケージ参照には `npm:` / `jsr:` を使います。裸のパッケージ名を任意の project の node_modules へ暗黙に接続しません。`node:` / `data:` は Node の処理を使います。
- ローカルファイルからの `file:` import など既存の Node 解決は維持します。HTTP(S) モジュールからの `file:` import は拒否します。
- CommonJS の直接配信（`.cjs` / `.cts`）は未対応です。HTTP(S) のソースは ESM として扱います。CommonJS の呼び出し元から同期 import する場合、top-level await のある ESM は Node の制約により読み込めません。

### キャッシュと安全性

非認証の URL ソースは `~/.tnpm/cache/modules-v1/` に、取得 URL・最終 URL・media type・SHA-256 とともに保存します。URL のハッシュをファイル名に使い、一時ファイルから atomic に置換します。再利用時は保存済み内容の checksum を検証します。Unix のキャッシュディレクトリは `0700`、ファイルは `0600` です。

- 通常は保存済みソースを再利用します。変更され得る URL を更新するには `run --no-cache FILE` を使います。この指定は HTTP ソースの永続キャッシュを読み書きしません。
- `run --offline FILE` は有効なキャッシュだけを使い、欠落・破損時は `ENOTCACHED` で失敗します。`--offline` と `--no-cache` は併用できません。
- URL 内の userinfo / パスワードを拒否します。認証は npmrc の URL スコープ設定だけを使用し、許可された origin / path の外へリダイレクトして認証情報を転送しません。認証付きソースは永続保存しません。
- `Cache-Control: no-store` のソースも永続保存しません。それ以外は URL をキャッシュスナップショットとして扱い、通常実行では max-age による自動更新を行いません。
- HTTPS モジュールの直接の HTTP import、HTTPS から HTTP へのリダイレクトを拒否します。HTTP は loopback の fixture や開発サーバーを含め利用可能です。

**URL ソースは project の `tnpm-lock.json` に固定しません。** 再現性が必要なときは版を含む不変 URL を指定するか、パッケージ依存としてインストールしてください。キャッシュの checksum は破損検出であり、配信者による署名や初回取得物の安全性を保証しません。

## 対応範囲

これらは `tnpm run FILE` / ファイル名ショートカットによる**直接実行**に適用します。任意の `node ...` を含む package script、worker、別プロセスに loader を自動注入しません。HTTP(S) URL 自体を run のエントリーに指定する機能、Deno API、deno.json の imports / import map、JSR への publish、型検査は対象外です。

URL / JSR / npm から取得したコードは通常の Node プロセスでユーザー権限で実行されます。スクリプト許可リストは install hook 用であり、import したコードを隔離するサンドボックスではありません。
