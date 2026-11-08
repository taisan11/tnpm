# `package.json`

tnpm が認識する manifest の主なフィールドを示します。`package.json` のトップレベルは JSON オブジェクトである必要があります。未知のフィールドは一般に保持しますが、**tnpm がその機能を実装していることを意味しません**。

## 基本フィールド

| フィールド | 型 | tnpm での用途 |
| --- | --- | --- |
| `name` | 文字列 | 公開・表示・workspace 識別に使用。`publish` では必須 |
| `version` | SemVer 文字列 | `version`・`publish`・workspace の範囲判定に使用 |
| `private` | 真偽値 | `publish` は `true` を拒否 |
| `description`、`license` | 文字列 | `init` で初期値を作成。公開 metadata にも利用 |
| `main` | 文字列 | `init` で初期値を作成、`pack` 時に原則同梱 |
| `bin` | 文字列または名前→パスのオブジェクト | インストール時に `node_modules/.bin`、グローバル配置時に実行用リンク / shim を作成 |
| `funding` | 文字列・オブジェクト・配列 | `fund` で URL を列挙 |
| `scripts` | 名前→コマンド文字列 | `run` と各種 lifecycle に使用 |
| `files` | パス・glob の配列 | `pack` / `publish` の同梱対象の選択 |

`init` は未指定の場合に `name`、`version`、`description`、`main`、`license` と `scripts.test` を設定します。既存のフィールドは残します。

## 依存関係

次の 4 フィールドは、パッケージ名から依存指定文字列へのオブジェクトです。

| フィールド | 意味 | 取り扱い |
| --- | --- | --- |
| `dependencies` | 通常依存 | `install` で解決・配置 |
| `devDependencies` | 開発依存 | `-D` で追加、`--omit=dev` / `--production` で配置を省略 |
| `optionalDependencies` | optional 依存 | `-O` で追加、不適合な OS / CPU 等では省略可能 |
| `peerDependencies` | ホスト側で必要な依存 | 既定で自動解決。`--legacy-peer-deps` で無効、`--strict-peer-deps` で不一致をエラー化 |
| `peerDependenciesMeta` | peer の付加情報 | `{ "name": { "optional": true } }` 形式の optional peer に対応 |

```json
{
  "name": "example",
  "version": "1.0.0",
  "dependencies": {
    "react": "^19.0.0",
    "util-alias": "npm:real-util@^2",
    "@luca/cases": "jsr:@luca/cases@^1"
  },
  "devDependencies": {
    "typescript": "^5.0.0"
  },
  "peerDependencies": {
    "react-dom": "^19.0.0"
  },
  "peerDependenciesMeta": {
    "react-dom": { "optional": true }
  }
}
```

対応する依存指定は SemVer（exact・`^`・`~`・比較式・OR・タグなど）、`npm:` エイリアス、`jsr:`、`file:`、ローカル tarball、HTTP(S) tarball、Git、`workspace:` です。**依存元と指定の組み合わせによって制限があります。** 詳細は [依存ソースと workspaces](../dependency-sources.md)、[JSR](../jsr-and-url-imports.md) を参照してください。

## ワークスペース・解決ルール

| フィールド | 型 | 仕様 |
| --- | --- | --- |
| `workspaces` | パターンの配列（または npm 形式の `packages` を持つオブジェクト） | ルート配下の workspace を探索。`*` / `?` / `**` と `!` の除外パターンに対応 |
| `overrides` | 文字列またはネストしたマッピング | **ルート manifest のみ**を解釈。条件付きキー、`.`、直接依存を指す `$name` をサポート |
| `trustedDependencies` | パッケージ名の配列 | **ルート manifest のみ**で依存 lifecycle の実行許可を追加。組み込み許可リストは残る |

```json
{
  "private": true,
  "workspaces": ["packages/*"],
  "overrides": {
    "some-dependency": "^2.0.0",
    "parent@^1": { "child": "^3.0.0" }
  },
  "trustedDependencies": ["my-native-addon"]
}
```

- workspace 全体で単一の `tnpm-lock.json` を使用します。`tnpm install -w app` のように対象 workspace を選べます。
- `trustedDependencies` は任意コードの実行を許す設定です。名前による許可であり、サンドボックスではありません。
- インストールの配置方式（`hoisted` / `isolated`）は **`package.json` ではなく `.npmrc` の `node-linker`** で設定します。

## スクリプトと実行環境

```json
{
  "scripts": {
    "dev": "vite --host",
    "build": "tsc && vite build",
    "prebuild": "echo preparing",
    "postbuild": "echo completed"
  }
}
```

`tnpm run build` では `prebuild` → `build` → `postbuild` を実行します（`--ignore-scripts` で前後のフックを停止）。インストールする依存の `preinstall` / `install` / `postinstall` は原則として信頼リスト制です。ルート・workspace のスクリプトと依存のスクリプトは扱いが異なります。

詳細: [スクリプト実行](./cli/execution.md)、[信頼設定](../security-and-diagnostics.md)。

## エンジン・プラットフォーム

| フィールド | 用途 |
| --- | --- |
| `engines.node`、`engines.tnpm` | バージョン条件。不一致時は通常警告、`--engine-strict` ではエラー |
| `os`、`cpu`、`libc` | 文字列または配列による OS / CPU / Linux libc 制約。除外条件も対応 |
| `gypfile`、`binding.gyp` | 条件により依存パッケージで `node-gyp rebuild` を実行 |
| `bundleDependencies` / `bundledDependencies` | **取得した tarball** 内に含まれる依存を解釈。tnpm 自身の `pack` で bundle を作る機能は未対応 |

`engines.npm` を tnpm のバージョンとして扱うことはありません。ネイティブビルドには外部のツールチェーンが必要な場合があります。

## パッケージ公開

```json
{
  "name": "@team/widget",
  "version": "1.2.3",
  "files": ["dist/**", "README.md"],
  "publishConfig": {
    "registry": "https://registry.example.com/",
    "tag": "next",
    "access": "public"
  }
}
```

`publishConfig` でサポートするキーは **`registry` / `tag` / `access` の文字列のみ**です。ほかのキーを指定するとエラーです。`private: true` は `publish` を禁止します。`pack` / `publish` のファイル選択には、`files`、ignore ファイル、強制除外ルールが適用されます。

詳細: [認証・公開コマンド](./cli/publishing.md)、[pack / publish の仕様](../publishing.md)。

## 非対応・注意点

- npm の `package-lock.json` は読み書きしません。`tnpm-lock.json` は tnpm 独自のフォーマットです。
- フィールドが manifest に存在しても、npm のすべての関連機能が tnpm に実装されるとは限りません。たとえば `packageManager` の宣言を使ったツールチェーン自動切り替えは提供していません。
- `npm:` エイリアスの入れ子や、ソース依存内の一部 `file:` 宣言などは未対応です。実装済み範囲は [互換性](../compatibility.md) を参照してください。
