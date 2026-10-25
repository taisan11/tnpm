# 依存グラフの連続配列化：変更前後の比較

2026-10-24、Apple M4 / macOS arm64、Node v26.12.1、両方とも Rust release ビルド。
各ケース15サンプル、warmup 2回、実行順は交互、並列数1、外れ値除外なし。

[Bun の Cache-Friendly Data Layout](https://bun.sh/blog/behind-the-scenes-of-bun-install#cache-friendly-data-layout) を参考に、lockfile の到達性検証・依存フラグの分類を次の構成に変更しました。

- パッケージを整数 ID で参照し、依存先・パッケージごとの辺オフセット・フラグを連続配列に格納。
- 依存名・SemVer 範囲は元の JSON から借用。文字列バッファへ再コピーする方式ではありません。
- 分類のホットループは依存先 ID と1 byteの辺フラグだけを読み、キューには整数 ID と伝播フラグを保存。
- 到達済み・分類済み状態をパッケージ ID 別の配列に保存し、走査中の JSON 複製とパス文字列の複製・再検索を削減。
- インストール対象を絞り込む際も lockfile の JSON 全体を複製せず、対象だけの借用スナップショットを構築。

## 平均実行時間

| ケース | パッケージ数 | 変更前 (ms) | 変更後 (ms) | 倍率 |
| --- | ---: | ---: | ---: | ---: |
| lockfile 処理のみ | 37 | 3.715 | 3.634 | 1.02x |
| lockfile 処理のみ | 100 | 3.897 | 3.467 | 1.12x |
| lockfile 処理のみ | 1,000 | 14.100 | 8.629 | 1.63x |
| lockfile 処理のみ | 5,000 | 61.470 | 29.896 | 2.06x |
| no-op install | 37 | 4.564 | 4.432 | 1.03x |
| no-op install | 100 | 5.987 | 5.221 | 1.15x |
| no-op install | 1,000 | 30.347 | 21.542 | 1.41x |
| no-op install | 5,000 | 148.504 | 99.260 | 1.50x |

小規模グラフはプロセス起動コストの比率が高く、37パッケージの差はほぼ横ばいと見なします。

## 測定条件・制約

- 合成のスコープ付き循環グラフ、最大4依存/パッケージ、production / dev / optional のルート。パッケージ数が同じでも実際の Hono / Wrangler プロジェクトではありません。
- コマンドは `install --offline --ignore-scripts --json`。lockfile 処理のみでは `--package-lock-only` を追加。
- プロセス起動、JSON 解析、グラフの構築・走査、lockfile 整形・書き込み、出力読み取りを含む wall-clock 時間です。グラフ走査だけのマイクロベンチマークではありません。no-op はファイルシステム走査と hidden lockfile の確認・書き込みも含みます。
- ネットワーク・tarball 展開・パッケージのコピー・lifecycle script は含みません。cold install や公開レジストリで同じ倍率になるとは限りません。
- 各回の入力復元と検証は計測外。変更前後の出力 lockfile と manifest の一致、no-op の installed=0 と hidden lockfile の一致を確認しました。
- 64通りの合成グラフについて旧方式との比較テストも追加し、辺ごとの物理解決、循環、ネスト、欠落した祖先、root の有無、依存指定の上書き、alias、不正名、peer / optional / devOptional の伝播、対象限定の到達性を検証します。
- 保存済み入力 lockfile を再利用する経路の比較であり、未取得パッケージのバージョン探索や peer set のバックトラッキング自体を配列化したものではありません。npm 互換 JSON の保存形式は変更していません。
- 検証: Rust 単体56件・通常のCLI結合59件・npm相互互換2件が成功。`cargo fmt --all -- --check` / `cargo clippy --all-targets -- -D warnings` も成功しました。

## 再現

```sh
node benchmarks/graph.mjs --before=target/tnpm-before-layout --sizes=37,100,1000,5000 --samples=15 --output=benchmarks/results/graph-layout.json
node benchmarks/graph.mjs --before=target/tnpm-before-layout --mode=noop --sizes=37,100,1000,5000 --samples=15 --output=benchmarks/results/graph-layout-noop.json
```

`target/tnpm-before-layout` は変更前の release バイナリです。再現には変更前ソースからビルドしたバイナリを別途用意してください。
生サンプル・p50 / p99・バイナリ SHA-256・出力 checksum は [lockfile 処理](graph-layout.json) / [no-op](graph-layout-noop.json) に保存しています。過去の公開レジストリ・Cloudflare Workers の測定結果は上書きしていません。
