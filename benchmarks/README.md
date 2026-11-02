# tnpm / Bun の CLI ベンチマーク

既存の CLI 参照系は Mitata 1.0.34 の `measure()`、install は削除・検証を計測外に分離するため `hrtime.bigint()` で、毎回新しい CLI プロセスを起動して実行時間を測ります。
Rust の実行時依存は追加しません。Mitata はこのディレクトリだけの開発依存です。

## 構成と実行入口

```text
benchmarks/
  *.mjs       計測スクリプト（従来の呼び出しパスを維持）
  lib/        fixture・ケース定義・process・stats の共通処理
  tests/      ベンチ基盤の検証。計測スクリプトは import / 実行しない
  results/    JSON 生データ・Markdown レポート・結果の索引
```

| npm script | 入口 | 公開ネットワーク |
| --- | --- | --- |
| `bench` | `bench.mjs`（Mitata CLI 比較） | 既定は loopback、`--live` のみ公開 npm |
| `bench:install` | `install.mjs` | GitHub / npm（warmup と cold ケース） |
| `bench:graph` | `graph.mjs --before=PATH` | なし |
| `bench:cache` | `cache.mjs --before=PATH` | loopback のみ |
| `bench:profile` | `profile-network.mjs` | 公開 npm |
| `bench:transport` | `compare-network.mjs` | 公開 npm |
| `test` | `tests/*.test.mjs` | 公開通信なし。Bun 本体のテストは通常 skip |

例: `npm --prefix benchmarks run bench:cache -- --before=/absolute/path/to/baseline`。下記の `node benchmarks/NAME.mjs` でも従来どおり実行できます。
各 runner の引数・測定順・統計・結果保存先は整理前から変更していません。[結果の索引](results/README.md)。

## ビルドと通常実行

```sh
# リポジトリのルートから
cargo build --release
npm --prefix benchmarks install --ignore-scripts --no-audit --no-fund
node benchmarks/bench.mjs
```

再現用の追加オプション:

```sh
node benchmarks/bench.mjs --samples=50
node benchmarks/bench.mjs --tools=tnpm,bun
node benchmarks/bench.mjs --tools=tnpm --output=benchmarks/results/tnpm-only
node benchmarks/bench.mjs --filter='warm cache' --output=benchmarks/results/cache
node benchmarks/bench.mjs --tnpm=/absolute/path/to/tnpm
node --test benchmarks/tests/*.test.mjs
TNPM_BENCH_BUN_TESTS=1 node --test benchmarks/tests/bun.test.mjs
```

## Cloudflare Workers テンプレートの install（全 CLI warm cache）

```sh
cargo build --release
node benchmarks/install.mjs
node benchmarks/install.mjs --samples=20 --warmup=3
# 取得済みのコミットで再現
node benchmarks/install.mjs --ref=110fb1c4dc34e050665ca89e0a2fa1a00fde29ef
node --test benchmarks/tests/install-fixture.test.mjs
```

- 対象は [honojs/starter の cloudflare-workers](https://github.com/honojs/starter/tree/main/templates/cloudflare-workers)。`main` を1回コミット SHA に固定してテンプレート全ファイルを取得し、Git blob checksum を検証します。元の `package.json` は変更しません。
- npm で v3 lockfile を作り Bun に渡します。npm は準備にのみ使い、計測しません。tnpm は warmup 中に独自に依存を解決します。各 CLI の lockfile は以後保持し、インストール結果の一致を検証します。
- HOME / キャッシュ / npmrc を両者で分離し、各 CLI・各ケースで2回以上 warmup。通常ケースは初回のみ公開レジストリへの通信を許し、計測は **両 CLI に `--offline`** を指定します。ただし lifecycle script の通信は制限していません。cold Bun + tnpm ケースは両方のキャッシュ削除後に毎回レジストリへ接続します。HTTP 回数は外部計測していません。
- `clean install / warm cache`: 各回 `node_modules` だけを削除し再構築します。削除時間は計測外です。
- `clean install / cold Bun + tnpm caches`: 各 Bun / tnpm 実行の前に両者の install cache 全体を削除して、レジストリから再取得します。Bun と tnpm の cold install を同じ条件で比較します。通信時間を含みます。
- `no-op install / warm cache`: `node_modules` も残して通常の install を実行します。再構築と同じケースに混ぜません。
- `--ignore-scripts` は指定せず、各 CLI の既定ポリシーで lifecycle script を実行します。Bun は信頼された依存のスクリプトのみを実行するため、両者で実行されるスクリプトが同じとは限りません。
- 既定10サンプル、並列数1、ラウンドごとに実行順をローテーション。`hrtime.bigint()` による subprocess wall-clock で、起動と出力読み取りを含みます。setup / warmup / 削除 / 検証は含みません。較正呼び出しや外れ値除外はありません。
- 毎回、インストール済み name/version の multiset が両者で一致すること、manifest が変わらないこと、Hono を実際に import できることを計測外で検証します。
- Bun と tnpm は展開済みキャッシュを利用しますが、キャッシュ形式・検証・clone/link の実装は異なります。OS のファイルキャッシュも warm です。
- 結果は `results/install-tnpm-bun-scripts.md` / `.json`。取得コミット・manifest・npm の基準 lockfile・実行コマンド・ツールのバージョン・tnpm バイナリの checksum・全サンプルを保存します。`--output=PREFIX` で別名に保存できます。

[過去のスクリプト無効時の実測結果](results/install-cloudflare-workers.md)

## 依存グラフのデータ配置を変更前後で比較（完全ローカル）

```sh
# 変更前の release バイナリを別名で保存してから、変更後をビルドします。
# BASELINE_BINARY は変更前バイナリのパスに置き換えてください。
cargo build --release
node benchmarks/graph.mjs --before=BASELINE_BINARY --output=benchmarks/results/graph-layout.json
node benchmarks/graph.mjs --before=BASELINE_BINARY --mode=noop --output=benchmarks/results/graph-layout-noop.json
```

- 同じ合成 tnpm lockfile を2つの tnpm バイナリへ渡し、起動から完了までを測定します。既定は100 / 1,000 / 5,000パッケージ、各15サンプル・warmup 2回です。`--sizes=37,100,1000,5000` / `--samples=N` / `--after=PATH` で変更できます。
- スコープ付き・循環・共有依存（最大4辺/パッケージ）と production / dev / optional のルートを含みます。実プロジェクトや新規バージョン解決のベンチマークではありません。
- `lock`（既定）は `install --package-lock-only`、`noop` は fixture に生成済みの `node_modules` を残す通常の `install`。どちらも `--offline --ignore-scripts --json` を指定し、ダウンロード・展開は行いません。
- 各回の入力 lockfile 復元、fixture 作成、結果検証は計測外。変更前後の順番を交互にし、manifest と出力 lockfile（全依存フラグを含む）の一致を毎回検証します。no-op は hidden lockfile の内容も検証します。
- HOME / npmrc を隔離します。OS のファイルキャッシュは warm、並列数1、外れ値は除外しません。保存する JSON はバイナリ SHA-256・条件・全サンプル・出力の SHA-256 を含みます。

[測定結果と制約](results/graph-layout.md)

## レジストリキャッシュ v2 / v3 の比較（ローカル HTTP）

```sh
# BASELINE_BINARY は変更前の release バイナリです。
cargo build --release
node benchmarks/cache.mjs --before=BASELINE_BINARY --output=benchmarks/results/cache-v3.json
node benchmarks/cache.mjs --before=BASELINE_BINARY --sizes=300,3000 --samples=15
```

- 同じ合成 packument を2つの tnpm へ配布し、互換性のない各キャッシュを別々の HOME に作成します。既定300 / 3,000バージョン、各15サンプル・warmup 2回、実行順は交互、並列数1です。`--after=PATH` で変更後バイナリも指定できます。
- `view` の完全一致・タグ・狭い範囲（3バージョン）・メタデータ全体、offline の依存解決、HTTP 304、cold cache の生成を分けます。表示内容・解決件数は毎回検証します。
- install は `--package-lock-only --no-save --offline --ignore-scripts --json`。lockfile がない1依存のプロジェクトで範囲を再解決します。tarball 取得・展開・node_modules 配置は含みません。
- HTTP はカウント可能な loopback サーバーのみです。warm ケースは0リクエスト、304 ケースは条件付き1件、cold は全文200を1件と毎回確認します。公開 npm レジストリには接続しません。
- cold のキャッシュ削除、fixture 作成、検証は計測外。プロセス起動・出力読み取り・`TNPM_PROFILE=1` の出力は含みます。OS ファイルキャッシュは warm、外れ値除外なしです。
- 同じ説明・キー・license などの重複と、トップレベルの time / readme を含む合成データです。実際の各パッケージで容量削減率や倍率が同じになるとは限りません。
- JSON レポートはバイナリ checksum、形式番号、キャッシュ容量、全サンプル、内部 profile、HTTP 件数を保持します。[測定結果](results/cache-v3.md)。

## `hono` を実ネットワークで測定

```sh
# 公開 npm レジストリへの実際の HTTPS 通信（2者比較、既定5サンプル）
node benchmarks/bench.mjs --live

# Bun / tnpm のみ・毎回取得するケースだけに絞る
node benchmarks/bench.mjs --live --tools=tnpm,bun --filter='force network'

# サンプル数と保存先を指定
node benchmarks/bench.mjs --live --samples=10 --output=benchmarks/results/hono-10
```

- 接続先は `https://registry.npmjs.org/`、対象は実在する `hono` です。測定器側でローカルサーバーや中継プロキシは設けません。OS のシステムプロキシがある場合は各 CLI の動作に従います。
- 測定前に選択した最初の CLI で最新バージョンを1回取得し、全 CLI の測定対象を同じ `hono@<version>` に固定します。実際のバージョンと解決に使った CLI を結果に記録します。
- バージョン取得・JSON 表示・整形表示のそれぞれを、通常実行と毎回取得で比較します。通常実行はウォームアップ後のキャッシュを使う可能性があります。
- 毎回取得は tnpm / Bun とも `--no-cache` を指定します。両 CLI の転送内容・圧縮方式は同一とは限りません。
- TLS、DNS、CDN、ネットワークの変動、プロセス起動時間を含む測定です。ツールごとの HTTP 回数やキャッシュヒットは直接観測していないため **N/A（不明）** と記録します。0件とみなしたり、無通信を保証したりしません。
- 無通信を検証できない warm-cache 専用ケースと、合成依存の一覧などローカル専用ケースは実行しません。`hono` 本体のインストールやスクリプト実行もしません。
- 個人の npmrc・認証情報・プロキシ環境変数は引き継がず、ツールごとに一時 HOME / キャッシュを作成して終了時に削除します。OS のシステムプロキシは reqwest の既定動作に従います。
- リクエスト数を抑えるため、実ネットワークモードの既定は5サンプルです。別途、各ケースのウォームアップ2回と Mitata の較正呼び出しも発生します。

結果は **`results/live-tnpm-bun.md` / `results/live-tnpm-bun.json`** に保存します。既存のローカル測定結果は上書きしません。

## 通信・解析時間の切り分け

`TNPM_PROFILE=1` で、キャッシュ読み込み・HTTP ヘッダー待ち・本文取得・JSON 解析・バージョン解決・出力などの時間を、`tnpm-profile: {JSON}` として stderr に記録します。通常の stdout と動作は変更しません。URL・認証トークン・本文は記録しません。

```sh
cargo build --release
node benchmarks/profile-network.mjs --label=enabled --output=benchmarks/results/profile-enabled
node benchmarks/profile-network.mjs --cache --label=cached --output=benchmarks/results/profile-cached
```

既定の対象は既存の比較と同じ `hono@4.13.13`、5サンプルです。`--spec=hono@<version>` / `--samples=N` / `--tnpm=PATH` で変更できます。本文の byte 数は自動展開後のサイズであり、圧縮時の実転送量ではありません。`headers_ms` は DNS・TCP・TLS・サーバー待ちの合計で、TLS だけの時間ではありません。

通常ビルドで reqwest のデフォルト機能と gzip / Brotli / deflate / zstd が有効です。調査時は同じバイナリの設定を切り替え、5条件（非圧縮HTTP/1.1、gzipのみ、非圧縮HTTP/2、gzip＋HTTP/2、全機能の通常設定）を比較できます。

```sh
cargo build --release
node benchmarks/compare-network.mjs
# 必要なら1条件だけ計測
node benchmarks/profile-network.mjs --transport=gzip --label=gzip --output=benchmarks/results/profile-gzip
```

5条件の順番を各ラウンドでローテーションし、全ケースで `--no-cache`・HTTP 200・新規 CLI / Client を検証します。結果は `results/registry-transport-comparison-enabled.md` / `.json` です。元の `live-hono.*` や旧4ビルド比較は上書きしません。切り替えは `TNPM_PROFILE=1` のときだけ有効で、通常実行の機能を落とすものではありません。

過去の結果にある `encoded_payload_bytes` は診断用 raw probe で計測した HTTP 本文の byte 数で、HTTP ヘッダーや TLS フレームを含む全 wire bytes ではありません。この probe は現在のリポジトリには含まれず、上記の手順では再計測できません。CLI 本体の所要時間とは直接比較しません。

通常ビルドは reqwest の `system-proxy` も有効です。fixture はプロキシ環境変数を除外しますが、OS のシステムプロキシ設定がある場合は reqwest の既定動作に従います。過去の調査レポートは変更前の構成を測った履歴として保持しています。

## 測定対象

| 用途 | tnpm | Bun |
| --- | --- | --- |
| 直接依存の一覧（テキスト） | `ls` | `bun pm ls` |
| 全依存の一覧（テキスト） | `ls --all` | `bun pm ls --all` |
| 依存一覧（JSON） | `ls --json` / `ls --all --json` | 対応なし・N/A |
| 支援先 | `fund --json` | 対応なし・N/A |
| バージョン情報（参照のみ） | `version --json` | 同等の JSON レポートなし・N/A |
| レジストリ設定取得 | `get registry` | 対応なし・N/A（`bun pm pkg get` は用途が違う） |
| 公開バージョン取得 | `view <pkg> version` | `bun info <pkg> version` |
| メタデータ（JSON） | `view <pkg> --json` | `bun info <pkg> --json` |
| メタデータ（整形表示） | `view <pkg>` | `bun info <pkg>` |

各レジストリ取得は通常動作 / warm cache / 毎回通信に分けます。対応しないコマンドを npm ラッパーやダミー処理で代用しません。

## 条件

- macOS / Linux、Node.js 20 以降。Bun は PATH 上、tnpm は `target/release/tnpm` を使用。既定は2者比較、`--tools` で測定対象を選べます。install ベンチの準備には npm も必要です。
- ローカルは既定30サンプル、実ネットワークは既定5サンプル・並列数1。極端な値の除外や複数呼び出しのバッチ化はしません。
- 各ケースで実行順をローテーションし、出力内容の検証と2回のウォームアップを測定前に行います。
- プロセス起動、CLI 処理、stdout/stderr の読み取りを含む wall-clock 時間。ビルド・fixture 作成・ウォームアップは含みません。
- ローカルモードの固定 fixture は直接依存20 / 総依存100。レジストリには合成した300バージョンの同一データを使用。
- ローカルモードのレジストリは loopback の HTTP サーバー。`--live` を付けない限り公開 npm レジストリには接続せず、TLS・インターネット遅延は含まれません。
- HOME、npm cache、Bun cache、npmrc、XDG 設定をツールごとの一時ディレクトリに分離。ユーザーの `BUN_*` も引き継ぎません。レジストリ URL にツール別パスを設け、User-Agent に依存せず HTTP 回数を計測します。既存のキャッシュ・トークン・設定は変更せず、終了時は今回作成した fixture だけ削除します。
- ローカルモードでは全ツール共通の fixture に npm lockfile を作成。Bun を選ぶ場合は測定前に `bun pm migrate` で `bun.lock` を生成し、通信が発生しないことを検証します。
- Bun の一覧は `bun.lock` ベース、tnpm はインストール済み依存を検査します。全く同じ内部処理ではないため、差を parser 単体の速度として解釈しないでください。テキスト一覧の検証では直接20 / 全100パッケージが出力されることを確認します。
- warm cache は Bun `--prefer-offline`、tnpm は通常キャッシュ。HTTP リクエストが0件であることを検証します。Bun は2回目のウォームアップでも通信した場合、同条件で比較できないため理由を付けて N/A にします。
- 毎回通信は tnpm・Bun とも `--no-cache`。HTTP 200 の全データを毎回取得することを検証します。
- 通常動作の `view` は HTTP 回数を併記し、異なるキャッシュポリシーを同じ条件と誤認しないようにします。
- OS のファイルキャッシュは warm。tnpm と Bun で実装範囲・出力は同一ではなく、完全互換の性能比較ではありません。

ローカルの結果は `results/tnpm-bun.md` と `results/tnpm-bun.json`（サンプル時間は ns）に保存します。実ネットワークは `results/live-tnpm-bun.*` に分離し、過去の結果は上書きしません。
平均・p50・p99・実際のコマンド・通信回数・対象外の理由を記録します。倍率は「他ツールの時間 / tnpm の時間」で、1未満なら他ツールの方が速いことを意味します。
過去の npm / tnpm の結果は履歴として保持し、今回の2者比較と混ぜません。
エラー終了した CLI は計測成功とみなさず、その時点で中断します。
