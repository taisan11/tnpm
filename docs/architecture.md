# プロジェクト構成

tnpm は単一の Rust バイナリです。CLI の互換性を保ちながら、コマンドの表示・操作と共通のパッケージ管理処理を分けています。ベンチマーク用の Node.js 依存は Rust 本体から独立しています。

```text
src/
  main.rs                  終了コード・エラーチェーン・最終行のビルド情報
  app.rs                   作業ディレクトリ・設定・runtime の準備とコマンド振り分け
  cli.rs                   引数とオプション
  config.rs                npmrc・環境変数・CLI 設定の優先順位
  config/http.rs           共通のレジストリ選択・スコープ別認証・TLS 設定
  commands/
    cache.rs               グローバルキャッシュの容量集計・全削除
    init.rs                ローカル package.json の初期化
    packages.rs            ls / fund とインストール済み依存の参照
    run.rs                 run コマンドと pre/post フック
    version.rs             version コマンド、ファイル更新・Git 操作
    view.rs                view コマンド、フィールド選択
  install/
    mod.rs                 install / ci / update / uninstall / exec / outdated の制御
    update.rs              update --latest の取得・宣言更新とエイリアス維持
    report.rs              更新前後の差分と結果表示（表 / JSON）
    context.rs             実行単位の設定・HTTP client・メタデータのメモ化
    resolver.rs            通常依存のバージョン解決と物理配置
    resolver/peers.rs       peer set の配置・バージョン探索
    store.rs               tarball の取得・検証・安全な展開・展開済みキャッシュ
    tree.rs                node_modules の走査・配置・削除・bin・フック、prune
  registry/
    mod.rs                 メタデータ取得・再検証・バージョン選択
    cache.rs               v3 バイナリ形式・共有文字列プール・SemVer 索引
    http_cache.rs          HTTP キャッシュポリシー
    transport.rs           通信設定と計測時の切り替え
  lockfile.rs              lockfile の読み書き・パス・依存グラフの検証
  lockfile/graph.rs        借用した依存情報・整数 ID・SoA 配列による到達性と分類
  manifest.rs              package.json の読み込みと検証
  semver.rs                SemVer の解析・比較・範囲・増分
  spec.rs                  パッケージ名とバージョン／タグ／範囲指定の解析
  util/style/
    mod.rs                 colored による stdout / stderr 別の色制御
    table.rs               Unicode 表示幅に基づく表の整形
    progress.rs            stderr の段階別進捗と安全な URL 表示
    view.rs                レジストリ情報の表示整形
  scripts.rs               共通のプロセス実行・フック・bin 探索
  fs.rs                    JSON の一時ファイル経由の保存と一意な識別子
  json.rs                  nojson を使う共通 JSON 表現
  profile.rs               任意の性能計測
build.rs                   ビルド時コミットの埋め込みと Git 変更の監視
tests/
  commands.rs              CLI 結合テストの実行入口（単一の Cargo test target）
  commands/                cache / config / install / peers / update / 表示などの機能別テスト
  support/mod.rs           一時プロジェクト・ローカル HTTP レジストリ・tarball fixture
benchmarks/
  *.mjs                    CLI 比較・install・graph・cache・通信の実行入口
  lib/                     fixture・ケース定義・subprocess 実行・統計の共通処理
  tests/                   ベンチ基盤の Node.js テスト（実測とは分離）
  results/                 過去の JSON 生データ・Markdown レポートと索引
docs/                      設計・仕様・開発手順
```

## 責務の境界

- `main` はプロセスの終了処理だけを担当し、`app` が初期化と振り分けを行います。
- `commands` は利用者向けの操作と表示を担当します。インストールに関係するコマンドは共通の処理手順が多いため、`install::run` にまとめています。
- `cache` はグローバルな `~/.tnpm/cache` と `~/.tnpm/exec` の保守なので、project 探索・npmrc 読み込み・async runtime の作成より前に分岐します。容量集計と削除は symlink を辿らず、他の `~/.tnpm` 内ファイルは対象外です。
- `install` 内の `resolver` は配置計画となる lockfile を生成し、`tree` が実際のファイルを変更します。`store` は取得・展開・キャッシュに限定し、manifest や lockfile の更新を担当しません。
- `registry` のキャッシュ実装と通信設定は内部モジュールです。他の機能は取得 API を使い、バイナリ形式に直接依存しません。
- `registry::Query` が全表示・フィールド指定・候補全件・最新1件を区別します。install は表示用メタデータを復元せず、`view` は参照するルートキーだけを要求できます。キャッシュは全レスポンスを保持するため、別のクエリでも同じ保存済みデータを使用します。
- `lockfile` は npm 互換 JSON を保持し、`lockfile/graph` が走査用の読み取り専用スナップショットを作ります。依存名・範囲は元の JSON から借用し、依存先・オフセット・フラグを別々の連続配列に保存します。分類が終わって借用を解放してから JSON のフラグを更新します。物理配置を変更する resolver へ永続的な索引を持ち込まず、索引の無効化漏れを避けます。
- `manifest` / `semver` / `spec` / `fs` / `scripts` はコマンド実装に依存しません。共通処理を追加するときは、特定のコマンドを経由して呼び出さないようにします。
- `util/style` は色、表、進捗および `view` の人間向け出力の整形をまとめます。コマンドの実行やデータ取得は担当しません。
- 単一バイナリ内の実装なので、外部公開用のライブラリ API や Cargo workspace は設けていません。内部サブモジュール間の公開範囲は必要な範囲に限定します。

## 変更する場所の目安

| 変更内容 | 主な対象 |
| --- | --- |
| コマンド／オプションの追加 | `cli.rs`、`app.rs`、`commands/` または `install/mod.rs` |
| CLI の色・表・進捗・view の整形 | `util/style/` |
| 解決結果・巻き上げ・peer の不具合 | `install/resolver.rs`、`install/resolver/peers.rs`、`lockfile.rs` |
| tarball・integrity・展開済みキャッシュ | `install/store.rs` |
| no-op install・hidden lockfile・bin・フック | `install/tree.rs`、`scripts.rs` |
| HTTP の取得・再検証・キャッシュ形式 | `registry/` |
| 互換性や終了コードの確認 | `tests/commands/` と `tests/support/` |

検証コマンドとローカル通信の方針は [開発・検証](development.md) を参照してください。
