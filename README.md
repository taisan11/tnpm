# tnpm

Rust 製の小さな npm 互換パッケージマネージャー。JSON は `nojson`、引数は `noargs`、エラー処理は `anyhow` を使用します。完全互換より速度優先です。

```sh
cargo build --release
./target/release/tnpm --help
```

## プロジェクト構成・開発

- `src/commands/`: 利用者向けコマンド、`src/install/`: 依存解決・取得・配置、`src/registry/`: 通信とメタデータキャッシュ。
- `tests/commands/`: 機能別の CLI 結合テスト、`tests/support/`: 共有 fixture。`tests/commands.rs` は実行入口です。
- `benchmarks/`: 実行スクリプト、`benchmarks/lib/`: 共通処理、`benchmarks/tests/`: ベンチ基盤のテスト、`benchmarks/results/`: 測定履歴。
- [構成と責務](docs/architecture.md) / [開発・検証手順](docs/development.md) / [ドキュメント一覧](docs/README.md)

## コマンド

```sh
tnpm init -y                     # package.json を既定値で作成
tnpm install                     # package-lock.json v3 / node_modules を生成
tnpm i react                     # dependencies に保存
tnpm add react                   # install のエイリアス
tnpm i typescript -D             # devDependencies に保存
tnpm i react@19.0.0 -E           # 完全一致バージョンを保存
tnpm ci --ignore-scripts         # lockfile を変更せずクリーンインストール
tnpm uninstall react             # remove / rm も同じ
tnpm outdated --json             # 直接依存の current / wanted / latest
tnpm outdated --plain            # 枠・色なし（通常は枠付きの表）
tnpm update                      # 宣言済みの範囲内で更新
tnpm update --latest             # major を含め latest へ更新し package.json も変更
tnpm update astro --latest       # 指定した直接依存だけ latest へ更新
tnpm prune --omit=dev            # 余剰依存・開発専用依存を削除
tnpm run build -- --watch        # prebuild / build / postbuild
tnpm x tsc --version             # ローカル bin、なければパッケージを取得
tnpm exec --package=typescript -- tsc --version
tnpm init my-app                 # my-app/package.json を初期化（通信なし）
tnpm create vite my-app --template react # create-vite を取得・実行、後続引数を転送
tnpm ls                         # 直接依存
tnpm ls --all                   # 推移的依存も表示
tnpm ls --depth=1 --json
tnpm ls @scope/package          # 名前で絞り込み
tnpm fund                       # 依存の支援 URL（ブラウザは開かない）
tnpm fund --json
tnpm version                    # プロジェクトと tnpm のバージョン
tnpm version patch              # package.json と lockfile を更新
tnpm version minor --no-git-tag-version
tnpm version prerelease --preid=beta
tnpm view @astrojs/cloudflare     # npm 風の整形表示
tnpm view @astrojs/cloudflare --json # メタデータ全体を JSON 表示
tnpm view @astrojs/cloudflare --no-cache # キャッシュを読み書きせず直接取得
tnpm view react version
tnpm view '@scope/package@next' dist.tarball --json
tnpm view 'react@^19' version --json
tnpm get registry
tnpm get registry cache
tnpm get --json
tnpm cache                        # キャッシュの種類別容量と合計
tnpm cache --json                 # バイト単位の容量・ファイル数
tnpm cache rm                     # tnpm の全キャッシュを削除
```

- 共通オプション: `--json`、`--prefix PATH`、`--registry URL`、`--userconfig PATH`、`--globalconfig PATH`、`--help`。通信コマンドには `--no-cache`。
- 出力: `update` は更新前後のバージョン表を表示します。更新不要なら `No packages were updated.`、`outdated` で対象がなければ `All dependencies are up to date.`。通信・キャッシュ・展開・配置は `[処理済み/確認済み]` の進捗を stderr の同じ1行で更新します（リダイレクト時は追記）。警告や結果表示は上書きしません。成功・失敗・ヘルプを問わず最後に `tnpm vX.X.X(ビルド時コミットハッシュ)` を stderr に出します（JSON の stdout は汚しません）。[出力形式と進捗](docs/cli-output.md)。
- 通常の `update` は宣言された SemVer 範囲内で更新します。`^1.2.3` なら major を越えませんが、`*` / `>=1` / OR 条件など、宣言自体が複数 major を許す場合は越えることがあります。`--latest` はこの範囲を越えて `latest` タグへ更新します。
- `install` / `i` / `add`: レジストリ依存を解決し、巻き上げ・競合時のネスト・スコープ付きパッケージ・循環参照に対応した `node_modules` を作成。互換な lockfile があれば再解決せず利用します。`-D` / `-O` / `-E` / `--no-save` / `--package-lock-only` / `--omit=dev,optional` / `--production` / `--ignore-scripts` に対応。
- `ci`: v3（v2 の `packages` も可）の lockfile と manifest・依存の整合性を検証後、`node_modules` を削除・再構築。`package.json` と `package-lock.json` は書き換えません。
- `uninstall` / `remove` / `rm`: 宣言と不要な推移依存・bin を削除。`update [PACKAGE...]` は宣言範囲を変えず、対象とその解決結果を更新。`outdated [PACKAGE...]` は直接依存を調べ、更新候補があれば終了コード 1。表示幅に合わせた表（Unicode 対応）と `--json` に対応し、`npm:` エイリアスは参照先の wanted / latest を調べます。`prune [PACKAGE...]` は通信せず余剰パッケージを削除。
- `update [PACKAGE...] --latest`: 直接依存（dependencies / devDependencies / optionalDependencies）を参照先の `latest` タグへ major を含めて更新し、解決成功後に `package.json` と lockfile を保存します。exact / `^` / `~` の形式と `npm:` エイリアス・依存区分を維持し、`-E` で exact を強制できます。`--no-save` / `--package-lock-only` も利用できます。既存 peer のバージョン不一致はデフォルトで警告して続行します。厳密に検証する場合は `--strict-peer-deps` を指定してください。実行時の API 互換性までは保証しません。
- `run [SCRIPT] [-- ARGS...]`: 省略時はスクリプト一覧。祖先の `node_modules/.bin` を PATH に追加し、pre/post フック、`--if-present`、子プロセスの終了コードをサポート。`--ignore-scripts` は pre/post のみ無効化し、明示したスクリプトは実行します。スクリプト名より後の引数は tnpm のオプションとして解釈しません。
- `exec` / `x`: ローカル bin を優先し、なければ `~/.tnpm/exec/` に隔離して自動取得・実行。`--package`（複数可）、`--call` / `-c` をサポート。確認プロンプトはなく `--yes` は互換フラグです。実行パッケージはキャッシュに固定し、`--no-cache` で再解決できます。
- `init [PATH] [-y]`: PATH は必ずローカルの作成先ディレクトリです（省略時は現在地）。必要ならディレクトリを作成し、package.json の既定値を補完し、既存のフィールドを維持します。`--prefix` 指定時の相対 PATH はその配下です。パッケージ取得や `create-*` 実行は行いません。
- `create NAME [ARGS...]`: `create-NAME` を取得・実行する独立したコマンドです。バージョン・タグ・スコープ付きパッケージを指定でき、NAME より後の引数は initializer へそのまま転送します（`--` は省略可）。tnpm 自体のオプションは NAME より前に指定します。NAME は必須で、`init` へのフォールバックはありません。
- `outdated` の通常表示は枠付き・端末上では色付きの表です。`--plain` は従来の枠なし・色なし形式、`--json` は JSON です。色は `colored` を使用し、リダイレクト時は既定で無効、`NO_COLOR` でも無効化できます。
- `peerDependencies`: 必須 peer の自動取得、親または祖先への配置、peer set の共有・分離、複数範囲の交差とバージョン再探索、循環 peer、optional peer の存在時検証に対応。バージョン不一致は直接依存・推移依存ともデフォルトで警告して続行します（`--force` 不要）。`--strict-peer-deps` で不一致を `ERESOLVE` に変更できます。通常の dependencies の SemVer 範囲、必須 peer の存在、不正な配置は引き続き検証します。`--legacy-peer-deps` / `--force` / `--omit=peer` にも対応します。[仕様と検証範囲](docs/peer-dependencies.md)。
- `ls`: `node_modules` を読み、巻き上げられた依存・スコープ付きパッケージ・循環参照に対応。必須依存の欠落やバージョン不一致は終了コード 1。デフォルトは直接依存のみ。
- `fund`: インストール済み依存を再帰的に調べ、文字列・オブジェクト・配列形式の `funding` を表示。
- `version`: 明示的な SemVer、`major`、`minor`、`patch`、`premajor`、`preminor`、`prepatch`、`prerelease`、`from-git` に対応。`package-lock.json` と `npm-shrinkwrap.json` があればルートのバージョンも同期。Git 管理下ではクリーンな状態を確認してコミット・注釈付きタグを作成します。`--no-git-tag-version` で無効化できます。`preversion` / `version` / `postversion` スクリプトは `--ignore-scripts` で無効化でき、同一バージョンへの更新は `--allow-same-version` で許可できます。
- `view`: デフォルトは npm 風の整形表示（概要・配布情報・依存・メンテナー・タグ・公開日時）。`--json` でメタデータ全体を取得できます。npm レジストリのタグ、明示バージョン、一般的な範囲指定（`^` / `~` / 比較演算子 / ワイルドカード / OR / ハイフン）と、ドット区切りフィールド・配列添字に対応。フィールド指定時は従来どおり指定した値だけを表示します。パッケージを省略するか `.` を指定するとローカルのパッケージ名を使用します。
- `get`: **package.json の取得ではなく npm 設定の取得**。優先順位は既定値 < global npmrc < user npmrc < project `.npmrc` < `npm_config_*` 環境変数 < CLI。`.npmrc` の `${VARIABLE}` / `${VARIABLE?}` 展開、`key[]` 配列、コメント、スコープ別レジストリ・認証に対応。トークン・パスワード・秘密鍵設定などは表示しません。[対応範囲と JSR 設定例](docs/npmrc.md)。
- `cache`: `~/.tnpm/cache`（レジストリの旧形式・tarball・展開済みパッケージも含む）と `~/.tnpm/exec` の容量を、直下の種類別・合計で表示します。ディスク割り当て量と通常ファイルの論理サイズ・件数を分け、`--json` は正確なバイト数を返します。CoW / hardlink の共有ブロックは重複計上される場合があり、実際に解放される容量とは限りません。
- `cache rm`: 上記2ディレクトリを確認なしで削除します。`--json` は削除したパス一覧です。npm のキャッシュ、プロジェクトの node_modules / hidden lockfile、その他の `~/.tnpm` 内ファイルは変更しません。シンボリックリンクは参照先を辿らず、`~/.tnpm` 自体がリンクなら拒否します。project / npmrc / `--prefix` に依存せず実行でき、未作成なら容量0・削除なしで成功します。install / exec と同時には実行しないでください。[出力仕様](docs/cli-output.md#cache)。
- `npm:` エイリアス依存: `"local-name": "npm:real-package@^1"` のような宣言を解決し、実体の名前を維持しながら `node_modules/local-name` に配置します。スコープ付きパッケージと `tnpm install 'alias@npm:real-package@^1'` に対応します。

## 依存 feature

- `mimalloc`: tnpm 全体のグローバルアロケータとして使用します。標準構成（mimalloc v3）で、`secure` / `v2` などの追加 feature は有効にしません。ビルドには C コンパイラが必要です。
- `colored`: 進捗、警告・エラー、表、結果メッセージ、末尾のビルド情報を装飾します。stdout / stderr ごとに端末判定し、JSON や `--plain` の表には色を混ぜません。
- `anyhow`: アプリケーションのエラーを統一し、処理の文脈と元の原因を保持します。子プロセスの終了コードは引き続き維持します。
- `phf`（`macros`）: コマンドのエイリアスと npmrc の固定キーを、コンパイル時に生成する Map / Set で検索します。ヘルプの表示順や、利用者が変更する設定の格納方法は維持します。
- `simdutf8`（標準 feature）: レジストリ応答の UTF-8 を `compat` API で検証し、本文全体を String へコピーせず `nojson` に渡します。バイナリキャッシュの共有文字列プールは `basic` API で一度だけ検証します。対応 CPU の SIMD をライブラリ側で選択し、CPU 固有のビルドフラグは強制しません。
- `xxhash-rust`（`xxh3` のみ）: レジストリキャッシュの偶発的な破損を XXH3-128 で検出します。認証用途ではなく、tarball の暗号学的な SRI 検証は変更しません。追加の CPU 固有ビルドフラグは強制しません。
- `reqwest`: デフォルト feature（HTTPS/rustls、HTTP/2、charset、system-proxy）に加え、`gzip`・`brotli`・`deflate`・`zstd` を有効化。対応形式を `Accept-Encoding` で送信し、レスポンスを自動展開します。`json` / `blocking` は有効にせず、JSON は引き続き `nojson` で処理します。
- `tokio`: `default-features = false`、直接指定は `rt`。通信に必要なネットワーク・タイマー feature は reqwest 経由で有効になります。シングルスレッド async runtime と展開用の blocking pool を使用します。
- `dirs`: `home_dir()` で `~/.tnpm/cache` の基準となるホームディレクトリを取得します。
- `httpdate`: HTTP の `Date` / `Expires` / `Last-Modified` を解析します（追加の依存先なし）。
- `futures-util`（`std` のみ）: 最大16件の並列メタデータ取得・tarball ダウンロード。
- `tar` / `flate2`: Rust 内で tar/gzip 展開。`sha2` / `sha1` / `base64`: SRI integrity 検証、キャッシュキー生成。
- `libc`: Linux の `FICLONE` ioctl。macOS は `clonefile` を直接呼び出し、どちらも clone 非対応時は通常コピーにフォールバックします。
- `unicode-width`（デフォルト feature なし）: `outdated` と `update` の表を文字列の表示幅で揃えます。

## インストールの高速化・lockfile

- [npm の package-lock.json 仕様](https://docs.npmjs.com/cli/v12/configuring-npm/package-lock-json)に従う `lockfileVersion: 3` と物理パス別の `packages` を読み書きします。v2 の `packages` も読み込めます。v1 は未対応で、`npm-shrinkwrap.json` はインストール時に参照しません。
- メタデータ・tarball を最大16並列で取得し、HTTP 接続を再利用。展開は blocking pool へ分離します。同じ name/range の解決は実行中に共有します。
- lockfile の到達可能性検証・依存フラグの分類は、借用した文字列と連続した依存先 ID・オフセット・フラグ配列で走査します。JSON 全体の複製やキュー内のパス文字列コピーを避け、インストール対象の絞り込みも JSON を複製せず行います。npm の lockfile 形式は変更しません。[変更前後のローカル測定](benchmarks/results/graph-layout.md)。
- tarball は `~/.tnpm/cache/tarballs-v1/` に保存し、再利用時も integrity を検証。認証付き通信と `--no-cache` ではキャッシュしません。`ci` も保存済み tarball を再利用できます。
- 展開済みパッケージは URL と integrity ごとに `~/.tnpm/cache/packages-v1/` に保存し、クリーン install では再解凍せず配置します。macOS の `clonefile`、Linux の `FICLONE` を試し、非対応 filesystem では通常コピーにフォールバックします。コピーオンライトで配置するため、install script による変更は共有キャッシュへ反映されません。認証付き・`--no-cache` の場合は読み書きしません。`--offline` でも展開済みキャッシュを利用できます。
- `--offline` は通信へのフォールバックを禁止します。必要なメタデータ・tarball がなければ `ENOTCACHED`。期限切れの保存済みメタデータも使用できます。`--no-cache` との併用はエラーです。
- `node_modules/.package-lock.json` を生成し、記載ディレクトリの存在・余剰ディレクトリ・更新日時を確認してから manifest 読み込みの代わりに使用します。手動でファイルの内容だけを変更した場合は npm と同様に hidden lockfile を削除してください。
- バージョンが変わらないパッケージは再展開せず、ツリーと bin が変わらなければリンクの再生成も省略します。必須 tarball の取得・検証・展開を staging で完了してから置き換えます（`ci` の先行削除・スクリプト失敗を含め完全なトランザクションではありません）。
- SHA-512 / SHA-256 / SHA-1 を検証し、不正な lockfile パス、tar 内のパス遡上・リンク・デバイス、symlink 経由のインストール先を拒否します。integrity がない外部 lockfile は検証なしで取得します。
- 依存の `preinstall` / `install` / `postinstall`、ルートの install / prepare 系フックを実行します。信頼しないパッケージには `--ignore-scripts` を指定してください。

調査時だけ `TNPM_PROFILE=1` と `TNPM_PROFILE_TRANSPORT=baseline|gzip|http2|both|default` を組み合わせると、圧縮・HTTP バージョンの条件を切り替えられます。通常実行は常にデフォルト機能と全圧縮形式を利用します。以前の調査用 Cargo feature `profile-gzip` / `profile-http2` は不要になりました。

## レジストリキャッシュ

`view` / install は `~/.tnpm/cache/v3/` にレジストリ情報と HTTP キャッシュポリシーをバイナリ保存します。共有文字列プール、復元時にソート不要なオブジェクト索引、SemVer 順の索引で、不要な文字列の保存・全バージョンの再ソート・復元を削減します。通常の install は範囲に合う最新の1バージョンだけ復元し、peer の候補探索では全候補も取得できます。`view FIELD...` は必要なルートキーだけ復元します。URL ごとに分離し、タグ・バージョン・フィールドの違いは同じキャッシュから解決します（[バイナリ仕様](docs/cache-format.md) / [変更前後の測定](benchmarks/results/cache-v3.md)）。旧 v1 / v2 は読み込まず、移行・削除もしません。旧形式しかない環境はオンラインで再取得してから `--offline` を使用してください。tarball / 展開済みキャッシュと npm の lockfile 形式は変更しません。

- 有効期限は `Cache-Control: max-age` を優先し、なければ `Expires` から計算します。`Date`・`Age`・リクエストの所要時間を引いた**受信時点の残り TTL と5分の長い方**を採用します。TTL 未指定・短い TTL・単独の `max-age=0` も、受信から最低5分は無通信で利用します。5分を超える残り TTL はそのまま維持し、上限は設けません。
- 最低5分はサーバーの短い TTL を延長する tnpm 独自ポリシーです。ただし `no-store`、`no-cache`、`must-revalidate`、不正な鮮度情報、未対応の `Vary` は優先し、下限で制限を緩和しません。`must-revalidate` がある応答はサーバーの TTL のまま扱います。
- 期限内は無通信で利用します。期限切れや `no-cache` では再検証し、`ETag` があれば `If-None-Match`、なければ有効な `Last-Modified` を `If-Modified-Since` として送ります。検証情報がなければ本文を取得し直します。キャッシュの読み込みでは5分の起点を更新しません。
- `304 Not Modified` は保存済み本文を再利用し、応答ヘッダーと保存時刻を更新します。全バージョンの JSON の再解析・再エンコードは行いません。`200 OK` なら新しい本文に置き換えます。
- `no-cache` は保存禁止ではなく毎回再検証する指定です。`no-store`・`Vary: *`・未対応の `Vary` は保存せず、同じ URL の旧エントリーも無効化します。`Vary: Accept, Accept-Encoding, User-Agent` は保存時のリクエスト値と比較します。
- `--no-cache` は読み込み・書き込みの両方を無効化し、HTTP にも `Cache-Control: no-cache, no-store` を付けます。既存のキャッシュは変更しません。
- 破損・未対応形式は再取得します。通信・再検証に失敗しても上記ポリシーの期限を過ぎた情報を返しません。`stale-while-revalidate` / `stale-if-error` による古い本文の利用は実装しません。保存失敗は警告だけで、取得結果は利用できます。
- 認証トークン付きリクエストは、利用者間での情報混在を防ぐためキャッシュを読み書きしません。トークンは保存しません。
- OS ユーザー専用の private cache として扱います。`private` は許可し、共有キャッシュ向けの `s-maxage` / `proxy-revalidate` は適用しません。
- 保存は同一ディレクトリ内の一時ファイルから置換。Unix では新規ディレクトリは `0700`、ファイルは `0600` です。
- npm の `cache` 設定とは別の、tnpm 専用キャッシュです。

## 対応範囲

npm 全体の完全な互換実装ではありません。workspaces、グローバルインストール、file/Git/URL 依存、bundled dependencies、lockfile のみでの一覧、`fund` のブラウザ選択、npm 内部の全設定既定値は未対応です。peer の機能は対応していますが npm Arborist のすべての配置・バージョン選択順との一致を保証するものではありません。os/cpu の optional 除外は対応、libc/engine 検証・node-gyp 自動実行・audit は未対応です。主に macOS/Linux 向けで Windows の bin shim は簡易版です。JSON は2スペースで再整形します。`fund --json` は名前でまとめたフラットな一覧です。スクリプトや Git 操作失敗後は、npm と同様に変更が残る場合があります。

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo test --test commands npm_and_tnpm_can_share_v3_lockfiles -- --ignored # Node.js/npm との相互互換テスト
```

テストのレジストリ通信にはローカル HTTP サーバーを使い、実際の npm レジストリには接続しません。

## ベンチマーク

npm / tnpm / Bun の CLI 実行時間を Mitata で比較できます。異なるコマンド体系と対応範囲を明示して測定します。手順・測定条件は [benchmarks/README.md](benchmarks/README.md) を参照してください。

`node benchmarks/bench.mjs --live` で、実際の公開 npm レジストリから `hono` を取得して比較できます。

`node benchmarks/install.mjs` は Hono starter の Cloudflare Workers テンプレートで bun / npm / tnpm の install を比較します。各キャッシュを warmup し、計測中は全 CLI を offline に固定。`node_modules` 再構築と no-op を別ケースで測ります。[実測結果](benchmarks/results/install-cloudflare-workers.md)。
