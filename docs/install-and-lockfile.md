# インストール・lockfile・ファイル配置

依存解決、npm lockfile 互換、キャッシュされた展開物の配置、インストール時の安全性と制約です。

## インストールの高速化・lockfile

- `tnpm-lock.json`（JSON、`tnpmLockfileVersion: 1`）を優先します。ない場合は [npm の `package-lock.json`](https://docs.npmjs.com/cli/v12/configuring-npm/package-lock-json) v2 / v3 を読み込み、`tnpm-lock.json` に変換します。独自形式はパッケージごとに `[version, resolved, integrity, その他のフィールド]` を1行に保存します。`package-lock.json` は書き換えないため、以後 npm で使う場合は別途同期が必要です。npm v1 と `npm-shrinkwrap.json` の読み書きは未対応です。`--package-lock-only` は `tnpm-lock.json` のみを更新します。
- メタデータ・tarball を最大16並列で取得し、HTTP 接続を再利用。展開は blocking pool へ分離します。同じ name/range の解決は実行中に共有します。
- lockfile の到達可能性検証・依存フラグの分類は、借用した文字列と連続した依存先 ID・オフセット・フラグ配列で走査します。JSON 全体の複製やキュー内のパス文字列コピーを避け、インストール対象の絞り込みも JSON を複製せず行います。npm の lockfile 形式は変更しません。[変更前後のローカル測定](../benchmarks/results/graph-layout.md)。
- tarball は取得時に integrity を検証して展開しますが、保存しません。展開済みキャッシュがなければ再取得が必要です。
- 展開済みパッケージは名前とバージョンをキーに `~/.tnpm/cache/packages-v2/` に保存し、クリーン install では再解凍せず配置します。保存時の integrity が lockfile と異なる場合は再利用しません。integrity がない場合は展開済みキャッシュを読み書きせず、毎回取得します。macOS ではディレクトリ単位の `clonefile` を試し、リンクや特殊ファイルがないことを検証します。失敗時はファイル単位にフォールバックします。Linux では `FICLONE` を試し、非対応 filesystem では通常コピーを使用します。コピーオンライトで配置するため、install script による変更は共有キャッシュへ反映されません。認証付き・`--no-cache` の場合は読み書きしません。`--offline` でも展開済みキャッシュを利用できます。旧 `packages-v1` / `tarballs-v1` は使用・削除しません。
- `prune` は不要な依存を `node_modules` から削除します。`prune --all` / `-a` は `node_modules` 全体を削除し、manifest・lockfile・共有キャッシュを維持します。
- `--offline` は通信へのフォールバックを禁止します。必要なメタデータ・tarball がなければ `ENOTCACHED`。期限切れの保存済みメタデータも使用できます。`--no-cache` との併用はエラーです。
- `node_modules/.package-lock.json` を生成し、記載ディレクトリの存在・余剰ディレクトリ・更新日時を確認してから manifest 読み込みの代わりに使用します。手動でファイルの内容だけを変更した場合は npm と同様に hidden lockfile を削除してください。
- バージョンが変わらないパッケージは再展開せず、ツリーと bin が変わらなければリンクの再生成も省略します。必須 tarball の取得・検証・展開を staging で完了してから置き換えます（`ci` の先行削除・スクリプト失敗を含め完全なトランザクションではありません）。
- SHA-512 / SHA-256 / SHA-1 を検証し、不正な lockfile パス、tar 内のパス遡上・リンク・デバイス、symlink 経由のインストール先を拒否します。integrity がない外部 lockfile は検証なしで取得します。
- 依存の `preinstall` / `install` / `postinstall`、ルートの install / prepare 系フックを実行します。信頼しないパッケージには `--ignore-scripts` を指定してください。

調査時だけ `TNPM_PROFILE=1` と `TNPM_PROFILE_TRANSPORT=baseline|gzip|http2|both|default` を組み合わせると、圧縮・HTTP バージョンの条件を切り替えられます。通常実行は常にデフォルト機能と全圧縮形式を利用します。以前の調査用 Cargo feature `profile-gzip` / `profile-http2` は不要になりました。
