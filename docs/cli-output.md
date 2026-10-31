# CLI の結果表示・進捗・ビルド情報

## update と --latest

- `update`: package.json の宣言範囲内で更新します。`^1.2.3` は 1.x 内、`~1.2.3` は 1.2.x 内、完全一致の `1.2.3` はそのバージョンを維持します。宣言は変更しません。
- `update --latest`: major を含めて参照先の `latest` タグへ更新し、宣言も保存します。対象名を指定した場合はその直接依存だけ宣言を更新します。
- `*` / `>=1` / 複数 major にまたがる OR 条件は、通常の update でも major を越える可能性があります。「宣言範囲内」と「常に同一 major」は異なります。

成功した更新は実際の node_modules の更新前後を比較し、追加・削除・推移的依存の更新も表示します。ネストした依存はインストール先を含む名前で区別します。

```text
Package  Before  After
app      1.0.0   1.1.0
Updated 1 package.
```

変化がなければ `No packages were updated.` と表示します。追加前は `MISSING`、削除後は `REMOVED` です。解決・配置に失敗した場合は成功結果を表示しません。

`--package-lock-only` では lockfile の更新前後を比較し、ディスクへ配置したとは表示しません。さらに `--no-save` を付けた場合は未保存の計画として扱います。

`update --json` の stdout は単一の JSON オブジェクトです。

```json
{
  "updated": 1,
  "mode": "installed",
  "changes": [
    {"name": "app", "from": "1.0.0", "to": "1.1.0", "location": "node_modules/app"}
  ]
}
```

mode は `installed` / `lockfile` / `planned`。追加・削除で存在しない側のバージョンは null です。更新なしでも `updated: 0` と空の changes 配列を返します。

インストール・更新系の `--json` 実行時は lifecycle スクリプトの標準出力も stderr に流し、stdout の JSON を保ちます。明示的な `run` / `exec` の実行結果は従来どおり stdout です。

## outdated の対象なし

通常は Unicode の枠付き表を表示し、列幅は色を付ける前の文字列で計算します。`--plain` は従来の枠なし・色なしの表示、`--json` は JSON です。JSON と plain を同時に指定した場合は JSON を優先します。

現在のバージョンが latest と一致する依存を除外し、残りがなければ `All dependencies are up to date.` と表示して終了コード 0 で終了します。`--json` の stdout は従来どおり `{}` です。

## init / create / add

- `add` は `install` のエイリアスです。`-D` / `-E` なども同じ意味で処理します。ヘルプでは `install (alias: i, add)` にまとめます。
- `init [PATH]`: 第1引数は必ずローカルの作成先です。省略すれば現在地、相対パスなら現在地または明示した `--prefix` の配下に package.json を初期化します。`init vite` でも `create-vite` を実行しません。
- `create NAME [ARGS...]`: initializer 専用の独立コマンドです。たとえば `create vite my-app` が `create-vite` を実行します。NAME の省略はエラーです。

## cache

```sh
tnpm cache
tnpm cache --json
tnpm cache rm
tnpm cache rm --json
```

`cache` は `~/.tnpm/cache` と `~/.tnpm/exec` の各ルート・直下の項目別・合計を枠なしの表で表示します。`Disk usage` は Unix では `st_blocks × 512`（ディレクトリやリンク自体の割り当ても含む）、その他の OS では通常ファイルの論理サイズによる推定です。`Logical size` は通常ファイルのサイズ合計、`Files` は通常ファイルの件数。シンボリックリンクの参照先は走査せず、リンク数は別途表示します。

容量表示は KiB / MiB / GiB の1024単位です。CoW clone / hardlink の共有ブロックを重複計上する場合があるため、ディスク割り当て量は削除で解放される容量の保証ではありません。集計中の変更を固定するスナップショットではなく、アクセス権などで読めない項目があればエラーにします。

`--json` はトップレベルに `bytes`（論理サイズ）、`diskBytes`（割り当て量／推定）、`files`、`symlinks` の合計と `paths` 配列を返します。各ルートも同じ集計フィールドと `path`、直下の内訳の `entries` を持ちます。内訳の数値はルートの数値に含まれているため、親子を重ねて加算しないでください。数値は丸めずバイト単位です。

`cache rm` は確認なしで上記2ルートだけを削除し、成功時は `Removed all tnpm caches.`、どちらも存在しなければ `No tnpm caches to remove.` を表示します。キャッシュルートや内部にあるリンクはリンク自体だけを削除し、`~/.tnpm` 自体がリンクなら操作を拒否します。削除はトランザクションではなく、途中でエラーになれば一部が削除済みの場合があります。install / exec との同時実行は避けてください。

JSON の削除結果は、実際に存在して削除したルートのパスだけを返します。

```json
{"removed": ["/home/user/.tnpm/cache", "/home/user/.tnpm/exec"]}
```

どちらも存在しなければ `{"removed": []}` です。容量確認・削除はディレクトリを新規作成しません。project、npmrc、`--prefix` に依存せず、npm のキャッシュやプロジェクトの node_modules / hidden lockfile は変更しません。

## 進捗ログ

インストール・更新系コマンドの進捗は、端末上では stderr の同じ1行を更新します。resolve / prepare / place などの段階が変わっても、新しい行を増やさず置き換えます。端末の横幅に収まらない詳細は `...` で省略し、折り返しによって古い表示が残らないようにします。

警告・エラー・結果表・子スクリプトの出力を出す前には進捗行を確定します。警告や末尾のバージョン表示を進捗で上書きしません。子スクリプトが無出力なら進捗行を確定せず、そのまま更新します。出力があれば即座に転送し、末尾に改行がない場合だけ補います。無条件な改行による空行は挟みません。

stderr をファイルやパイプにリダイレクトした場合、または `TERM=dumb` の場合は、制御シーケンスを出さず従来どおり追記します。以下はリダイレクト時の例です（端末ではこれらが1行ずつ置き換わります）。

```text
tnpm: [0/1] resolve: checking app@^1
tnpm: [0/1] resolve: HTTP GET metadata https://registry.example/app
tnpm: [1/1] resolve: resolved app@^1
tnpm: [1/2] resolve: checking child@1
tnpm: [0/2] prepare: HTTP GET tarball https://registry.example/app.tgz
tnpm: [1/2] prepare: prepared node_modules/app
tnpm: [1/2] place: installed 1.1.0 at node_modules/app
```

処理済み件数／確認済み件数は段階別です。

- `resolve`: 一意なパッケージ名＋要求範囲のメタデータ解決。推移的依存や peer 条件の発見で分母が途中で増えます。同一要求の再利用は重複加算しません。
- `prepare`: 配置が必要な物理パスの準備。通信・展開済みパッケージの clone・展開を区別します。optional のスキップや失敗も処理済みとして記録します。
- `place`: 準備できたパッケージの実際の配置。変更不要ならその旨を表示します。
- `remove`: 不要になったパッケージの削除。
- `scripts`: 実際に存在する lifecycle フックの開始・完了・失敗。分母は見つかったフックの数です。

`[0/0]` のメッセージは、その段階に個別の処理対象がまだないときの状態説明です。キャッシュを使ったときに HTTP 通信をしたかのようなログは出しません。HTTP URL のユーザー情報・query・fragment、認証ヘッダーはログに含めません。

## 色

`colored` で進捗・表・警告・エラー・結果・ビルド情報を装飾します。端末幅の計算と省略処理を先に行うため、色の制御シーケンスで幅はずれません。

stdout / stderr は別々に端末判定します。JSON の stdout をファイルへ保存していても、stderr の進捗は端末上で色付き表示できます。リダイレクト先は既定で無色です。`NO_COLOR` が設定されていれば無色、`CLICOLOR=0` でも通常の色を無効にします。`CLICOLOR_FORCE=1` は色を強制しますが、`NO_COLOR` と `outdated --plain` の表は優先して無色を維持します。JSON のデータは常に無色です。

## 最終行

通常出力を flush した後、stderr の最終行に必ず次の形式を表示します。

```text
tnpm v0.1.0(abcdef0)
```

成功・エラー・ヘルプ・`--version`・子コマンド失敗で共通です。既存の終了コードは変更しません。進捗・末尾表示は JSON に混ぜないため、`tnpm update --json > result.json` のように stdout だけ保存できます。

コミットは `build.rs` がビルド時に Git の HEAD から取得して埋め込みます。実行先プロジェクトの Git 状態や実行時の環境変数からは取得しません。HEAD／refs／packed-refs の変更を Cargo の再実行条件に含め、worktree にも対応します。コミットのないリポジトリや Git がないソース配布からのビルドでは `unknown` と表示します。
