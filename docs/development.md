# 開発・検証

## ビルドと静的チェック

リポジトリルートで実行します。Rust 2024 edition に対応するツールチェーンが必要です。

```sh
cargo build
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

ソースの責務と変更箇所の目安は [プロジェクト構成](architecture.md) を参照してください。

`build.rs` がビルド時の短縮 Git HEAD を `TNPM_BUILD_COMMIT` として埋め込みます。コミットが存在しない場合や Git が使えない場合は `unknown` です。CLI はどのコマンドでも最後に `tnpm vVERSION(COMMIT)` を stderr に出します。[出力仕様](cli-output.md)も参照してください。

## Rust テスト

- 単体テストは対応するモジュールの `#[cfg(test)] mod tests` に置きます。
- CLI 結合テストの入口は `tests/commands.rs`、実体は `tests/commands/` の機能別モジュールです。`tests/support/mod.rs` の `Fixture` を共有し、テストごとにプロジェクト・HOME・キャッシュ・npmrc を分離して、終了時に片付けます。新規モジュールは `tests/commands/mod.rs` に登録してください。Cargo の実行 target は従来どおり `commands` です。
- レジストリ通信には loopback の HTTP サーバーを使います。実際の公開 npm レジストリを必要とするテストを通常テストに追加しないでください。
- tarball の展開先、symlink、integrity、子プロセスの終了コードなど、安全性と外部から観測できる挙動も確認します。
- Unix の端末表示テストは疑似端末を使用し、進捗の1行更新、警告・改行なしスクリプト出力・末尾表示の保持、`TERM=dumb` の通常ログへの切り替えを確認します。
- 同じ保証を単体・結合テストで繰り返さず、境界条件を担当する層で検証します。旧実装の複製や全組み合わせの網羅より、期待値が明確な代表ケースを優先します。レジストリの複数バージョンは `Fixture::releases` で用意できます。

機能別に実行する場合はモジュール名で絞り込めます。

```sh
cargo test --test commands commands::cache::
cargo test --test commands commands::peers::
cargo test --test commands commands::registry::
```

Node.js / npm を必要とする相互互換テストは `tests/commands/npm_compat.rs` に分離し、既定で無効です。これらもローカルレジストリのみを使います。

```sh
cargo test --test commands npm_and_tnpm_can_share_v3_lockfiles -- --ignored
cargo test --test commands npm_and_tnpm_share_scoped_cyclic_peer_sets_and_optional_peer_metadata -- --ignored
cargo test --test commands commands::npm_compat:: -- --ignored # 2件まとめて実行
```

## ベンチマーク用テスト

ベンチマークは `benchmarks/package.json` / `package-lock.json` で管理する独立した Node.js パッケージです。本体の Cargo 依存には追加しません。実測の入口は `benchmarks/*.mjs`、共通の fixture / process / stats は `benchmarks/lib/`、テストは `benchmarks/tests/` です。

```sh
npm --prefix benchmarks install --ignore-scripts --no-audit --no-fund
npm --prefix benchmarks test
```

全テスト、または個別の基盤テストを直接実行することもできます。

```sh
node --test benchmarks/tests/*.test.mjs
node --test benchmarks/tests/fixtures.test.mjs
node --test benchmarks/tests/stats.test.mjs
# Bun 本体を必要とする任意テスト（通常は skip）
TNPM_BENCH_BUN_TESTS=1 node --test benchmarks/tests/bun.test.mjs
```

通常の Node.js テストは loopback HTTP と fixture / モックだけを使い、公開レジストリには接続しません。`live.test.mjs` も実ネットワークの性能測定ではありません。
引数生成・出力検証だけのテストには `benchmarks/tests/support.mjs` の設定モックを使い、実サーバーや依存ツリーの生成は fixture 自体の検証に限定します。

実測はテストと分け、[ベンチマーク手順](../benchmarks/README.md) に従います。`--live`、install の warmup、通信 profile / transport 比較は公開サービスに接続するため、通常の検証では実行しません。過去の `benchmarks/results/` は測定履歴として保持し、[結果の索引](../benchmarks/results/README.md) から参照できます。
