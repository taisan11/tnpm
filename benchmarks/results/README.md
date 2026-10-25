# 測定結果の索引

ここは実測履歴です。JSON は生サンプル・条件を保持し、Markdown は解釈と制約を記録します。過去の結果は移動・削除・再整形せず保持します。新しい測定を保存するときは `--output` で別名を指定してください。ファイル名の `latest` は現行バイナリの再測定を意味しません。

## 実装変更の比較

| 対象 | レポート | 生データ |
| --- | --- | --- |
| レジストリキャッシュ v2 → v3 | [cache-v3.md](cache-v3.md) | [cache-v3.json](cache-v3.json) |
| 依存グラフのデータ配置 | [graph-layout.md](graph-layout.md) | [lockfile 処理](graph-layout.json) / [no-op](graph-layout-noop.json) |

## CLI / install

| 対象 | レポート | 生データ |
| --- | --- | --- |
| npm / tnpm / Bun のローカル CLI 比較 | [with-bun.md](with-bun.md) | [with-bun.json](with-bun.json) |
| Cloudflare Workers テンプレート install | [install-cloudflare-workers.md](install-cloudflare-workers.md) | [install-cloudflare-workers.json](install-cloudflare-workers.json) |
| 公開 npm の Hono メタデータ取得 | [live-hono.md](live-hono.md) | [live-hono.json](live-hono.json) |
| 初期の npm / tnpm 比較 | [latest.md](latest.md) | [latest.json](latest.json) |

## 通信・キャッシュの調査履歴

- [レジストリ調査](registry-investigation.md)
- [ベースライン profile](registry-profile-baseline.md) / [JSON](registry-profile-baseline.json)
- [キャッシュ profile](registry-profile-cached.md) / [JSON](registry-profile-cached.json)
- [圧縮有効時の profile](registry-profile-default-compression.md) / [JSON](registry-profile-default-compression.json)
- [transport 比較](registry-transport-comparison.md) / [JSON](registry-transport-comparison.json)
- [payload サイズの記録](registry-payload-size.json)

異なる実行・キャッシュ形式・機能・ネットワーク条件の結果を、そのまま同条件の比較として混ぜないでください。実行手順と現行の測定条件は [benchmarks/README.md](../README.md) を参照してください。
