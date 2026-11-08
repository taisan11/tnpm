# CLI・ランナー・スクリプト

## CLI とコマンドの分岐

```text
main.rs ─► cli.rs (parse)
             │
             ▼
         app.rs (root / config / workspaces)
             │
    ┌────────┼─────────────────────────────┐
    ▼        ▼                             ▼
 commands/   install::run             runner/ + scripts
 (get, view,  (install, ci,           (run, exec, create,
 auth, pack,   update, audit,           JS/TS file)
 cache...)    prune...)
```

`src/main.rs` は終了処理・エラーチェーン・ビルド情報表示、`src/app.rs` は作業ディレクトリと設定を決めてコマンドを振り分けます。`src/cli.rs` は alias（`i`、`add`、`rm`、`x`、`why`）とオプションを解析します。

`cache` / `alias` / `upgrade` はプロジェクトや設定の読み込みより前に専用経路へ分岐します。`-g` のグローバル操作は `src/global.rs` に渡します。

`run`、`exec`、`create`、スクリプト名ショートカットでは**対象の後ろは子プロセス用の引数**です。この解析ルールは [CLI リファレンス](/cli) に記載しています。

## package scripts

`src/scripts.rs` は外部プロセス、ライフサイクルフック、bin 探索を担当します。`src/commands/run.rs` は `package.json` の script を参照し、`node_modules/.bin` を PATH に加え、`pre<SCRIPT>` → 本体 → `post<SCRIPT>` を実行します。

依存パッケージの install lifecycle は `src/install/trust.rs` の名前ベース許可リスト（組み込み・ルート manifest・CLI の和集合）を通します。これは**権限隔離ではありません**。root / workspace 自身のフックとは別扱いです。

`src/commands/workspace_run.rs` は selected workspaces を逐次・並列・依存順で実行します。`--topological` は選択内の通常・開発・optional 依存の順序を使用し、peer は実行順制約に含めません。

## JS / TS 直接実行

`tnpm run ./src/main.ts` のようにファイルを指定すると、Node.js の同期 module hooks を使って直接実行します。

```text
local JS/TS entry
    │
    ▼
runner/mod.rs + loader.mjs
    │
    ├─ Node 標準解決を優先
    ├─ runner/oxc.rs: import 解析・TS/JSX 変換・解決
    ├─ runner/commonjs.rs: SWC による CommonJS module 変換
    └─ runner/remote.rs: HTTP(S), jsr:, npm: の取得・キャッシュ
    │
    ▼
Node.js runtime (実行・標準入出力・終了コード)
```

- Node.js 22.15+（または Node 23.5+）が必要です。
- Oxc による変換は型検査ではありません。`tsconfig.json` の一部パス alias を解決しますが、transform options や source map の全般的サポートはありません。
- HTTP(S)・`jsr:`・`npm:` import は**直接ファイル実行**に適用されます。`package.json` の任意の `node` シェルコマンドには挿入しません。
- ランナーのリモートソースはプロジェクトの `tnpm-lock.json` に固定されません。再現性が必要なときは不変 URL などを使用します。
- Worker や独立した Node 子プロセスへ hooks は自動伝播しません。

[ランナー仕様](../runner.md) / [JSR・HTTP import](../jsr-and-url-imports.md) / [依存スクリプトの信頼設定](../security-and-diagnostics.md)

## 出力・テスト

`src/util/style/` は端末の色・表・進捗・`view` の整形を担います。stdout の機械可読出力と stderr の診断を分離します。

CLI の主要な確認は `tests/commands/` にある統合テストで行います。共有 fixture は `tests/support/mod.rs` の loopback HTTP レジストリを使用し、通常のテストで外部 npm / GitHub を必要としません。

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

[開発と検証](../development.md)
