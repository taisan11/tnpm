# CLI リファレンス

```text
tnpm [OPTIONS] <COMMAND|SCRIPT> [ARGS...]
```

引数なし、または `-h` / `--help` でヘルプを表示します。組み込みコマンドとその別名を優先し、一致しない最初の名前は `package.json` のスクリプトとして `run` と同様に実行します。組み込みと同名のスクリプトは `tnpm run install` のように指定します。存在しないスクリプトはエラーです。`run` と同様に、スクリプトが存在せず指定が対応する JS / TS ファイル名の場合はファイルを実行します（[ランナー](runner.md)）。

## 引数の解釈

- 値を取るオプションは `--prefix path` と `--prefix=path` の両形式を使えます。`-p` / `-c` も別引数または `-p=value` / `-c=value` で指定できます。
- `run`、`exec` / `x`、`create` の最初の対象、および省略形のスクリプト名より後は、オプションを含めて子プロセスに転送します。直後の `--` は取り除きます。tnpm 自体のオプションは対象より前に置いてください。
- 明示的な `--` 以降は tnpm のオプションとして解釈しません。`run -- test --watch` のように対象を `--` の後に置くこともできます。それ以外のコマンドでは転送引数が使用できない場合があります。
- `ls` / `prune` の `--all`、`ls` の `--depth`、`version` 固有、`update --latest`、`outdated --plain` は対応コマンドの**後**に指定します。その他の共通オプションは対象の前に置けます。
- パーサーが受理することとコマンドに効果があることは異なります。下表の「対象」は実際にその設定を利用する機能です。未知のオプションはエラーです。

```sh
tnpm --prefix ./app run --ignore-scripts build -- --watch
tnpm --prefix ./app build --watch   # 同じく --watch はスクリプトに渡す
tnpm exec --package=typescript -- tsc --version
tnpm create vite my-app --template react
tnpm ls --depth=1 --json
```

## コマンド

| コマンド | 内容 |
| --- | --- |
| `install [PACKAGE...]`、`i`、`add` | 依存を解決・配置。パッケージ指定時は宣言にも追加。 |
| `uninstall PACKAGE...`、`remove`、`rm` | 指定した宣言と不要な依存を削除。 |
| `ci` | `package.json` と `tnpm-lock.json` の整合性を検証してクリーンインストール。パッケージ指定と `--package-lock-only` は不可。 |
| `update [PACKAGE...]` | 既存の宣言範囲で依存を更新。`--latest` は直接依存の宣言も更新。 |
| `outdated [PACKAGE...]` | 直接依存の current / wanted / latest を表示。更新候補がある場合は終了コード 1。 |
| `prune [PACKAGE...]` | 不要な依存を除去。`--all` は `node_modules` 全体を削除。 |
| `ls [PACKAGE...]` | インストール済み依存を一覧表示。通常は直接依存のみ。 |
| `fund` | インストール済み依存の funding URL を一覧表示。ブラウザは開かず、パッケージ引数も未対応。 |
| `run [SCRIPT] [ARGS...]` | スクリプトと pre/post フックを実行。対象省略時はスクリプト一覧。 |
| `exec [OPTIONS] [BINARY] [ARGS...]`、`x` | ローカル bin を優先し、必要ならパッケージを取得して実行。 |
| `create [OPTIONS] NAME [ARGS...]` | `create-NAME` initializer を取得・実行。NAME は必須。 |
| `init [PATH]` | 指定ディレクトリに `package.json` を初期化。PATH はローカルパス。 |
| `version [RELEASE]` | バージョン一覧を表示、または SemVer / `major` / `minor` / `patch` / `premajor` / `preminor` / `prepatch` / `prerelease` / `from-git` に更新。 |
| `view [PACKAGE] [FIELD...]` | レジストリのメタデータを表示。PACKAGE 省略または `.` はローカルの名前を使用。フィールドはドット・配列添字を指定可能。 |
| `get [KEY...]` | npm 設定を表示。認証情報などの秘密は非表示。 |
| `cache [rm]` | tnpm キャッシュ容量を表示、または `rm` で全 tnpm キャッシュを削除。 |
| `upgrade` | tnpm 自身を GitHub Release の最新安定版へ更新。引数なし。 |

詳細な制約や例は [コマンドリファレンス](commands.md)、出力形式は [CLI 出力](cli-output.md) を参照してください。

## 共通オプション

| オプション | 対象・動作 |
| --- | --- |
| `-h`, `--help` | ヘルプを表示して終了。スクリプト等の対象より後では子プロセスに渡す。 |
| `-V`, `--version` | tnpm 自体のバージョンを表示して終了。`version` コマンドとは別。 |
| `--json` | 対応コマンドの結果を JSON で出力。`run` は一覧表示に適用し、`exec` / `create` は子プロセスの出力形式を変更しない。 |
| `--prefix PATH` | プロジェクトディレクトリを指定。`init PATH` の相対 PATH は prefix を基準にする。`cache` と `upgrade` はプロジェクト非依存。 |
| `--registry URL` | レジストリを使用するコマンドの URL を指定。スコープ設定・認証の詳細は [npmrc](npmrc.md)。 |
| `--userconfig PATH` | 読み込む user npmrc を指定。 |
| `--globalconfig PATH` | 読み込む global npmrc を指定。 |
| `--no-cache` | `view` や install 系のレジストリ／パッケージキャッシュを読み書きせず、`exec` / `create` は再解決。install 系では `--offline` と併用不可。 |

設定の優先順位は既定値 < global npmrc < user npmrc < project npmrc < `npm_config_*` 環境変数 < CLI です。全オプションが npmrc 設定に対応するわけではありません。

## 依存の操作・実行オプション

| オプション | 対象・動作 |
| --- | --- |
| `-D`, `--save-dev` | `install PACKAGE...` の保存先を `devDependencies` にする。 |
| `-O`, `--save-optional` | `install PACKAGE...` の保存先を `optionalDependencies` にする。`-D` と両方指定した場合はこちらを優先。 |
| `-E`, `--save-exact` | `install PACKAGE...` や `update --latest` で完全一致バージョンを保存。 |
| `-S`, `--save` | 互換用に受理するフラグ。追加の動作はない。 |
| `--no-save` | install / uninstall / update で manifest と lockfile を保存しない。依存の配置は通常どおり行う。 |
| `--package-lock-only` | install / uninstall / update で解決・保存のみ行い、`node_modules` を変更しない。名前に反して npm の `package-lock.json` は扱わず、`tnpm-lock.json` を使う。`--no-save` 併用時は保存もしない。`ci` / `prune` では不可。 |
| `--omit TYPES` | install / ci / update / prune で `dev`、`optional`、`peer` の配置を省略。カンマ区切りを指定可能（例 `--omit=dev,optional`）。lockfile から依存を削除する指定ではない。 |
| `--production` | install 系で dev 依存の配置を省略。 |
| `--ignore-scripts` | install 系ではライフサイクルフックを無効化。`run` では明示スクリプトを実行し pre/post のみ無効化。`version` でもフックを無効化。 |
| `--legacy-peer-deps` | install 系で peer 依存の解決・検証を無効化。 |
| `--strict-peer-deps` | install 系で peer バージョン不一致を警告でなくエラーにする。 |
| `-f`, `--force` | install 系で strict 指定時も peer バージョン不一致を許可。必須 peer の欠落などは許可しない。 |
| `--offline` | install 系でネットワーク取得にフォールバックしない。必要なキャッシュがなければエラー。`--no-cache` とは併用不可。 |
| `--if-present` | `run` で指定したスクリプトがない場合に成功する。 |
| `-y`, `--yes` | 互換用フラグ。`init` は元から非対話で既定値を使用し、`exec` / `create` も確認プロンプトを出さない。 |
| `-p`, `--package PACKAGE` | `exec` / `create` の実行用パッケージを指定。複数回指定可能。 |
| `-c`, `--call COMMAND` | `exec` / `create` で実行するコマンド文字列を指定。 |

## コマンド固有オプション

| オプション | 対象・動作 |
| --- | --- |
| `-a`, `--all` | `ls`: 推移的依存を含む。`prune`: `node_modules` 全体を削除。`prune` ではパッケージ指定、`--omit`、`--production` と併用不可。 |
| `--depth N` | `ls` の最大表示深度を非負整数で指定。既定は直接依存のみ（パッケージ名による絞り込みや `--all` 時を除く）。 |
| `--latest` | `update` で直接依存を `latest` タグへ更新し、宣言範囲も書き換える。 |
| `--plain` | `outdated` の表を枠・色なしで表示。 |
| `--preid ID` | `version` の prerelease 識別子を指定。 |
| `--no-git-tag-version` | `version` 更新時に Git のコミット・タグ作成を行わない。 |
| `--allow-same-version` | `version` で現在と同じバージョンの指定を許可。 |

## 出力と終了コード

通常の結果は stdout、進捗・警告・エラーと最終ビルド情報は stderr です。終了コードは成功時 0、エラーや更新候補のある `outdated` は 1。`run` 等で実行した子プロセスの失敗コードは可能な範囲で引き継ぎます。色は端末ごとに判定し、`NO_COLOR` で無効化できます。[出力形式の詳細](cli-output.md)。
