# TypeScript ツールチェーン（tsrs）

tnpm は、スクリプトの内容やインストール済みの依存を変更せずに、**package.json の scripts 内の tsc コマンド**を [tsrs](https://github.com/oxc-project/tsrs) に置き換えられます。

たとえば次のようなスクリプトが対象です。

```json
{
  "scripts": {
    "check": "tsc --noEmit",
    "build": "tsc -p tsconfig.build.json"
  }
}
```

`tnpm run check`、`tnpm check`、`tnpm run build` は、従来どおり指定した名前のスクリプトを実行します。スクリプトとその `pre` / `post` フックの実行中、tnpm は一時的な `tsc` shim を `PATH` 上の `node_modules/.bin` **より前**に置きます。shim は tsrs を実行し、引数と終了ステータスをそのまま引き継ぎます。

このリポジトリのリリースワークフローは、バージョンを固定したネイティブ tsrs コンパイラと、それに付属する TypeScript 標準ライブラリのファイルを tnpm 実行ファイルに埋め込みます。初回使用時にユーザーのキャッシュへ展開します。`TNPM_TSRS_BUNDLE` を指定しない Rust ビルドでは、別途インストールされた `tsrs` 実行ファイルを代わりに使います。どちらもなければ、スクリプトは元の `tsc` を実行します。

## 設定

プロジェクトまたはユーザーの `.npmrc` に次を記述します。

```ini
toolchain-tsrs=auto
```

| 値 | 動作 |
| --- | --- |
| `auto`（既定） | インストール済みの `typescript` が 7.1.x で、tsrs が利用可能な場合のみ `tsc` を置き換える |
| `on` | tsrs が利用可能なら、TypeScript のバージョンにかかわらずスクリプトからの `tsc` 呼び出しを置き換える |
| `off` | `tsc` を置き換えない |

ほかの npm 互換設定と同様に、`npm_config_toolchain_tsrs=on` は `.npmrc` より優先されます。tsrs が同梱もインストールもされていなければ、`on` でも `tnpm run` は通常の動作を維持します。

現在の tsrs は固定された TypeScript 7.1 beta に追従する実験的な実装です。`auto` は意図的に保守的な動作とし、従来の TypeScript 5.x / 6.x プロジェクトでは既存のコンパイラを維持します。古いプロジェクトで明示的に試す場合は `on` を使い、診断と出力ファイルを比較してください。

## 互換性と制限

- `--noEmit`、通常のビルド、watch / build モードは tsrs が処理します。
- tsrs は現在 `--init` と `--showConfig` を実装していません。これらのコマンドライン操作は、元の `tsc` が利用可能ならそちらへフォールバックします。
- スクリプトの内容は書き換えません。シェルの連続したコマンドや入れ子のスクリプト呼び出しでも PATH 上の shim を使えます。`node node_modules/typescript/bin/tsc` のような明示的なパスは shim を**使いません**。
- 対象は `tnpm run` が起動するスクリプトとその pre/post フックだけです。依存インストール時の lifecycle フック、`tnpm exec tsc`、`import "typescript"` は書き換えません。
- 埋め込みコンパイラのバージョンは、プロジェクトにインストールされた TypeScript とは独立しています。完全な互換性は保証しません。
- tsrs の同梱により公式 tnpm バイナリのサイズは増加します。実行時の展開にはローカルのユーザーキャッシュを使い、ネットワーク通信は行いません。

埋め込みの tsrs コンパイラと TypeScript 標準ライブラリの型定義は Apache-2.0 ライセンスで提供され、著作権はそれぞれの貢献者および Microsoft Corporation に帰属します。帰属情報は [tsrs LICENSE and NOTICE](https://github.com/oxc-project/tsrs) を参照してください。リリース用のバンドルには、上流が提供する通知・ライセンスファイルも同梱します。

## メンテナー向けビルド

独自にビルドしたコンパイラを埋め込むには、対象プラットフォームのネイティブ `tsrs`（または `tsrs.exe`）と `lib.d.ts` をルートに含む `tar.gz` アーカイブを用意し、その絶対パスを `TNPM_TSRS_BUNDLE` に設定します。リリースワークフローは `.github/scripts/bundle_tsrs.py` でこのアーカイブを作成し、保存先を Cargo に渡します。通常の `cargo build` は小さなバイナリを生成し、取得処理は行いません。
