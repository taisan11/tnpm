# コマンドリファレンス

対応するコマンドと挙動の詳細です。[全オプションと引数の解釈](cli.md) も参照してください。最初に使う場合は [README](../README.md) を参照してください。

## コマンド一覧

workspaces / ローカル・Git 依存 / overrides / engines / ネイティブ依存 / `-g` の詳細は [workspaces と依存ソース](dependency-sources.md) を参照してください。

```sh
tnpm init -y                     # package.json を既定値で作成
tnpm install                     # tnpm-lock.json / node_modules を生成
tnpm i react                     # dependencies に保存
tnpm add react                   # install のエイリアス
tnpm i typescript -D             # devDependencies に保存
tnpm i react@19.0.0 -E           # 完全一致バージョンを保存
tnpm ci --ignore-scripts         # lockfile を使ってクリーンインストール
tnpm uninstall react             # remove / rm も同じ
tnpm outdated --json             # 直接依存の current / wanted / latest
tnpm outdated --plain            # 枠・色なし（通常は枠付きの表）
tnpm update                      # 宣言済みの範囲内で更新
tnpm update --latest             # major を含め latest へ更新し package.json も変更
tnpm update astro --latest       # 指定した直接依存だけ latest へ更新
tnpm upgrade                     # tnpm 自体を最新の GitHub Release に更新
tnpm alias create                # npm を tnpm のシェルエイリアスにする
tnpm alias delete                # tnpm が追加したエイリアス設定を削除
tnpm prune --omit=dev            # 余剰依存・開発専用依存を削除
tnpm prune --all                 # node_modules 全体を削除（-a も可）
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
- 出力: `update` は更新前後のバージョン表を表示します。更新不要なら `No packages were updated.`、`outdated` で対象がなければ `All dependencies are up to date.`。通信・キャッシュ・展開・配置は `[処理済み/確認済み]` の進捗を stderr の同じ1行で更新します（リダイレクト時は追記）。警告や結果表示は上書きしません。成功・失敗・ヘルプを問わず最後に `tnpm vX.X.X(ビルド時コミットハッシュ)` を stderr に出します（JSON の stdout は汚しません）。[出力形式と進捗](cli-output.md)。
- 通常の `update` は宣言された SemVer 範囲内で更新します。`^1.2.3` なら major を越えませんが、`*` / `>=1` / OR 条件など、宣言自体が複数 major を許す場合は越えることがあります。`--latest` はこの範囲を越えて `latest` タグへ更新します。
- `install` / `i` / `add`: レジストリ依存を解決し、巻き上げ・競合時のネスト・スコープ付きパッケージ・循環参照に対応した `node_modules` を作成。`tnpm-lock.json` を使用し、npm の `package-lock.json` は読み書きしません。`--no-save` では生成しません。互換な lockfile があれば再解決せず利用します。`-D` / `-O` / `-E` / `--no-save` / `--package-lock-only` / `--omit=dev,optional` / `--production` / `--ignore-scripts` に対応。
- `ci`: lockfile と manifest・依存の整合性を検証後、`node_modules` を削除・再構築。既存 lockfile と `package.json` は書き換えません。`tnpm-lock.json` が必要です。
- `uninstall` / `remove` / `rm`: 宣言と不要な推移依存・bin を削除。`update [PACKAGE...]` は宣言範囲を変えず、対象とその解決結果を更新。`outdated [PACKAGE...]` は直接依存を調べ、更新候補があれば終了コード 1。表示幅に合わせた表（Unicode 対応）と `--json` に対応し、`npm:` エイリアスは参照先の wanted / latest を調べます。`prune [PACKAGE...]` は通信せず余剰パッケージを削除。
- `update [PACKAGE...] --latest`: 直接依存（dependencies / devDependencies / optionalDependencies）を参照先の `latest` タグへ major を含めて更新し、解決成功後に `package.json` と lockfile を保存します。exact / `^` / `~` の形式と `npm:` エイリアス・依存区分を維持し、`-E` で exact を強制できます。`--no-save` / `--package-lock-only` も利用できます。peer の不一致は既定で警告し、`--strict-peer-deps` でエラーにできます。実行時の API 互換性までは保証しません。
- `upgrade`: [GitHub Release](https://github.com/taisan11/tnpm/releases) の最新の安定版を取得し、GitHub API の SHA-256 digest を検証してから現在の実行ファイルを置き換えます。引数はなく、同じまたは古いバージョンへの置換は行いません。`--json` に対応。macOS/Linux は一時ファイルから置換、Windows は実行中の旧ファイルを退避し、終了後に別プロセスで後片付けします。インストール先の書き込み権限が必要です。対応バイナリとリリース手順は [開発手順](development.md#リリース) を参照してください。
- `alias create` / `alias delete`: Unix では `$SHELL` が bash、zsh、fish の場合、そのシェルの設定に `alias npm='tnpm'` を追加 / tnpm が追加した部分のみ削除します。対象は bash の `~/.bashrc`、zsh の `${ZDOTDIR:-$HOME}/.zshrc`、fish の `${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish` です。設定ファイルが読み込まれる新しい対話シェルで反映され、スクリプトや非対話シェルの npm は置き換えません。
- Windows では `tnpm.exe` と同じディレクトリに `npm.cmd` / `npm.ps1` と復元用ファイルを作成し、ユーザー PATH の先頭に移します。管理者権限は不要です。既存ファイルは上書きせず、削除時には作成したファイルだけを消します。PATH が作成後に変更されていなければ元に戻し、変更済みならそのまま残します。システム PATH はユーザー PATH より優先されるため、システム PATH 内の npm やシェルのエイリアスが優先される場合があります。PowerShell の実行ポリシーによっては `npm.ps1` を実行できません。変更後は新しいターミナルを開いてください。元の npm 本体は変更せず、tnpm は npm の完全互換ではありません。
- `run [SCRIPT] [-- ARGS...]`: 省略時はスクリプト一覧。祖先の `node_modules/.bin` を PATH に追加し、pre/post フック、`--if-present`、子プロセスの終了コードをサポート。`--ignore-scripts` は pre/post のみ無効化し、明示したスクリプトは実行します。スクリプト名より後の引数は tnpm のオプションとして解釈しません。
- `exec` / `x`: ローカル bin を優先し、なければ `~/.tnpm/exec/` に隔離して自動取得・実行。`--package`（複数可）、`--call` / `-c` をサポート。確認プロンプトはなく `--yes` は互換フラグです。実行パッケージはキャッシュに固定し、`--no-cache` で再解決できます。
- `init [PATH] [-y]`: PATH は必ずローカルの作成先ディレクトリです（省略時は現在地）。必要ならディレクトリを作成し、package.json の既定値を補完し、既存のフィールドを維持します。`--prefix` 指定時の相対 PATH はその配下です。パッケージ取得や `create-*` 実行は行いません。
- `create NAME [ARGS...]`: `create-NAME` を取得・実行する独立したコマンドです。バージョン・タグ・スコープ付きパッケージを指定でき、NAME より後の引数は initializer へそのまま転送します（`--` は省略可）。tnpm 自体のオプションは NAME より前に指定します。NAME は必須で、`init` へのフォールバックはありません。
- `outdated` の通常表示は枠付き・端末上では色付きの表です。`--plain` は従来の枠なし・色なし形式、`--json` は JSON です。色は `colored` を使用し、リダイレクト時は既定で無効、`NO_COLOR` でも無効化できます。
- `peerDependencies`: 既定で必須 peer を自動取得し、peer set の共有・分離とバージョン再探索を行い、不一致は警告します。`--legacy-peer-deps` で解決を無効化でき、`--strict-peer-deps` では不一致を `ERESOLVE` にします。[仕様と検証範囲](peer-dependencies.md)。
- `ls`: `node_modules` を読み、巻き上げられた依存・スコープ付きパッケージ・循環参照に対応。必須依存の欠落やバージョン不一致は終了コード 1。デフォルトは直接依存のみ。
- `fund`: インストール済み依存を再帰的に調べ、文字列・オブジェクト・配列形式の `funding` を表示。
- `version`: 明示的な SemVer、`major`、`minor`、`patch`、`premajor`、`preminor`、`prepatch`、`prerelease`、`from-git` に対応。`tnpm-lock.json` があればルートのバージョンも同期。Git 管理下ではクリーンな状態を確認してコミット・注釈付きタグを作成します。`--no-git-tag-version` で無効化できます。`preversion` / `version` / `postversion` スクリプトは `--ignore-scripts` で無効化でき、同一バージョンへの更新は `--allow-same-version` で許可できます。
- `view`: デフォルトは npm 風の整形表示（概要・配布情報・依存・メンテナー・タグ・公開日時）。`--json` でメタデータ全体を取得できます。npm レジストリのタグ、明示バージョン、一般的な範囲指定（`^` / `~` / 比較演算子 / ワイルドカード / OR / ハイフン）と、ドット区切りフィールド・配列添字に対応。フィールド指定時は従来どおり指定した値だけを表示します。パッケージを省略するか `.` を指定するとローカルのパッケージ名を使用します。
- `get`: **package.json の取得ではなく npm 設定の取得**。優先順位は既定値 < global npmrc < user npmrc < project `.npmrc` < `npm_config_*` 環境変数 < CLI。`.npmrc` の `${VARIABLE}` / `${VARIABLE?}` 展開、`key[]` 配列、コメント、スコープ別レジストリ・認証に対応。トークン・パスワード・秘密鍵設定などは表示しません。[対応範囲と JSR 設定例](npmrc.md)。
- `cache`: `~/.tnpm/cache`（レジストリの旧形式・tarball・展開済みパッケージも含む）と `~/.tnpm/exec` の容量を、直下の種類別・合計で表示します。ディスク割り当て量と通常ファイルの論理サイズ・件数を分け、`--json` は正確なバイト数を返します。CoW / hardlink の共有ブロックは重複計上される場合があり、実際に解放される容量とは限りません。
- `cache rm`: 上記2ディレクトリを確認なしで削除します。`--json` は削除したパス一覧です。npm のキャッシュ、プロジェクトの node_modules / hidden lockfile、その他の `~/.tnpm` 内ファイルは変更しません。シンボリックリンクは参照先を辿らず、`~/.tnpm` 自体がリンクなら拒否します。project / npmrc / `--prefix` に依存せず実行でき、未作成なら容量0・削除なしで成功します。install / exec と同時には実行しないでください。[出力仕様](cli-output.md#cache)。
- `npm:` エイリアス依存: `"local-name": "npm:real-package@^1"` のような宣言を解決し、実体の名前を維持しながら `node_modules/local-name` に配置します。スコープ付きパッケージと `tnpm install 'alias@npm:real-package@^1'` に対応します。
