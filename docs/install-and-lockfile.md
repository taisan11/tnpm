# インストール・lockfile・ファイル配置

依存解決、独自 lockfile、キャッシュされた展開物の配置、インストール時の安全性と制約です。

## インストールの高速化・lockfile

- `tnpm-lock.json`（JSON、`tnpmLockfileVersion: 1`）を使用します。パッケージごとに `[version, resolved, integrity, その他のフィールド]` を1行に保存します。npm の `package-lock.json` と `npm-shrinkwrap.json` は読み書きしません。`--package-lock-only` は `tnpm-lock.json` のみを更新します。
- メタデータ・tarball を最大16並列で取得し、HTTP 接続を再利用。展開は blocking pool へ分離します。同じ URL のメタデータは実行中に1回だけ取得・解析し、タグ・範囲・フィールドの違いはその結果から解決します。
- lockfile の到達可能性検証・依存フラグの分類・peer 検査は、借用した文字列と連続した依存先 ID・オフセット・フラグ配列を共有する1つの索引で行います。JSON 全体の複製やキュー内のパス文字列コピーを避け、インストール対象の絞り込みも JSON を複製せず行います。変更がなく検証できる lockfile は、git / tarball ソース（`resolved` が固定）を含む場合も再解決せず再利用します。workspace / `file:` ソースは内容が変わり得るため毎回再解決します。npm の lockfile 形式は変更しません。[変更前後のローカル測定](https://github.com/taisan11/tnpm/blob/main/benchmarks/results/graph-layout.md)。
- レジストリ tarball は取得時に integrity を検証して展開しますが、元 archive を保存しません。展開済みキャッシュがなければ再取得が必要です。直接 URL ソースの元 archive だけは `source-tarballs-v1` に URL / integrity をキーに保存し、offline 再配置に利用します。
- 展開済みパッケージは名前とバージョンをキーに `~/.tnpm/cache/packages-v2/` に保存し、クリーン install では再解凍せず配置します。保存時の integrity が lockfile と異なる場合は再利用しません。integrity がない場合は展開済みキャッシュを読み書きせず、毎回取得します。macOS ではディレクトリ単位の `clonefile` を試し、リンクや特殊ファイルがないことを検証します。失敗時はファイル単位にフォールバックします。Linux では `FICLONE` を試し、非対応 filesystem では通常コピーを使用します。コピーオンライトで配置するため、install script による変更は共有キャッシュへ反映されません。認証付き・`--no-cache` の場合は読み書きしません。`--offline` でも展開済みキャッシュを利用できます。旧 `packages-v1` / `tarballs-v1` は使用・削除しません。
- `prune` は不要な依存を `node_modules` から削除します。`prune --all` / `-a` は `node_modules` 全体を削除し、manifest・lockfile・共有キャッシュを維持します。
- Linux の reflink 対応は install ごとにコピー元・コピー先のデバイスの組み合わせで1回確認し、並列のパッケージ配置でも結果を共有します。`FICLONE` 非対応（`EOPNOTSUPP` / `ENOTTY`）や別 filesystem（`EXDEV`）の場合は、以後ファイルごとの clone 試行・一時ファイルの作成と削除を省いて直接コピーします。確認用ファイルは1回だけ作成・削除します。一時的なエラーは非対応と判断せず、従来どおり clone 失敗時にコピーします。既定の hoisted 配置ではハードリンクを使わず、依存ファイルの直接編集・bin の権限変更・lifecycle script が共有キャッシュを変更しない保証を維持します。`node-linker=isolated` のハードリンク配置と制約は変更しません。
- `--offline` は通信へのフォールバックを禁止します。必要なメタデータ・tarball がなければ `ENOTCACHED`。期限切れの保存済みメタデータも使用できます。`--no-cache` との併用はエラーです。
- `node_modules/.tnpm-lock.json` を生成し、記載ディレクトリの存在・余剰ディレクトリ・更新日時を確認してから manifest 読み込みの代わりに使用します。手動でファイルの内容だけを変更した場合は npm と同様に hidden lockfile を削除してください。
- バージョンが変わらないパッケージは再展開せず、ツリーと bin が変わらなければリンクの再生成も省略します。必須 tarball の取得・検証・展開を staging で完了してから置き換えます（`ci` の先行削除・スクリプト失敗を含め完全なトランザクションではありません）。
- SHA-512 / SHA-256 / SHA-1 を検証し、不正な lockfile パス、tar 内のパス遡上・リンク・デバイス、symlink 経由のインストール先を拒否します。integrity がない外部 lockfile は検証なしで取得します。
- 宣言された workspace だけは検証済みソースへの symlink を許可し、競合する子依存をその workspace の node_modules に配置します。`file:` のコピー、Git のコミットと integrity、bundled package の archive 内パスも独自 lockfile に保持します。[依存ソースの仕様](dependency-sources.md)。
- 依存の `preinstall` / `install` / `postinstall` とソースの prepare / 暗黙の node-gyp は許可リスト制です。Bun 由来の既定リストにルートの `trustedDependencies` / `--trust` を追加し、許可した依存とルート / workspace のフックだけ実行します。`--ignore-scripts` はすべて停止します。hidden lockfile の `tnpmBuilds` に成功記録を持ち、後から許可した依存や `--ignore-scripts` 後の未ビルド依存を次回 install で実行できます。[信頼設定と rebuild](security-and-diagnostics.md)。

## 配置方式と共有ストア

従来の巻き上げ配置は `node-linker=hoisted`（既定）です。分離インストールは `node-linker=isolated` で有効にします。

```sh
tnpm install --node-linker=isolated
tnpm ci --node-linker=isolated --offline
```

ユーザー共通の `~/.npmrc` に指定すると全プロジェクトの既定値を変えられます。プロジェクトの `.npmrc`、`npm_config_node_linker`、CLI の `--node-linker` の順に上書きできます。

```ini
node-linker=isolated
store-dir=~/.tnpm/cache
```

- `isolated`: `node_modules/.tnpm/<id>/node_modules/<package>` に各依存の実体を配置し、プロジェクト直下と各パッケージから、宣言済みの依存だけをディレクトリリンクで参照します。peer と optional 依存も解決済み lockfile に従います。
- `store-dir`: 共有パッケージストアの親ディレクトリ（既定 `~/.tnpm/cache`）。展開済み取得用は `packages-v2/`、分離配置の内容アドレス付きファイル用は `files-v1/` です。分離配置ではファイル単位で **`fs::hard_link` のみ**を使い、CoW や通常コピーへのフォールバックはしません。未登録のファイルは SHA-256 で検証・登録し、既存の正常な object はコピー・fsync せず再利用します。`-g` とは別機能です。
- 通常のレジストリ依存で lifecycle hook の実行が不要な場合、`files-index-v1/` のパッケージ別索引から staging へ直接ハードリンクし、巻き上げ配置のコピーを経由せず分離配置を構築します。索引は integrity・取得 URL・lockfile のパッケージ情報に結び付け、相対ファイルパス・SHA-256 object 名・空ディレクトリ・bin の実行属性を保持して atomic に更新します。warm install は object の種別・サイズ・更新日時・権限（Unix ではデバイスと inode も）を確認し、変更のない object の再ハッシュを省きます。リンク数の変更でも変わる ctime は使いません。欠損・変更を検出した場合は展開済みキャッシュまたは integrity を検証した tarball から補修し、正常な object は再発行しません。索引があれば `packages-v2/` を削除した後でも offline 配置できます。ローカルキャッシュのメタデータを信頼する最適化であり、ファイル内容と更新日時などを意図的に同時改ざんする攻撃の検出は保証しません。
- 許可された依存 hook またはルート hook が動く場合は、従来どおり独立した hoisted staging で全 hook を実行した後にハードリンク化します。hook は別パッケージを書き換えられるため、そのパッケージだけでなく依存グラフ全体をこの経路に戻します。ビルド済みの内容を未実行のパッケージ索引へ保存しません。workspace・`file:` / Git / URL・bundled 依存、認証付き取得や `--no-cache` も従来経路を使用します。
- 配置方式の切り替え時と分離配置への再インストール時は `node_modules` を再構築します。lockfile の形式は変わりません。
- 分離配置では現在 `rebuild` と通常の `prune` に未対応です。`tnpm install` で再配置するか、`tnpm prune --all` で全削除してください。依存 lifecycle script は現時点では hoisted の一時配置で実行後、分離リンクへ変換されます。
- Windows の依存ディレクトリリンクには、特別なシンボリックリンク権限を要求しない NTFS Junction（`mklink /J` 相当）を使用します。Linux/macOS はディレクトリ symlink です。ファイルのハードリンクには Windows の NTFS と同じボリューム、または Unix の同一ファイルシステムが必要です。異なるボリュームに設定した場合はコピーで代替せずエラーを返します。
- ハードリンクは同じファイル実体を共有するため、インストール後にパッケージファイルを**直接編集するとストアや別プロジェクトにも影響します**。将来のインストールで既存ストアオブジェクトを再利用するときは SHA-256 で再検証し、破損していれば書き直します。新規登録はコピーしながら1回だけハッシュするため、登録直後の再ハッシュは行いません。Unix ではストアへの登録時に書き込み権限を外します。Windows はリンク先の改変防止を保証しません。

調査時だけ `TNPM_PROFILE=1` と `TNPM_PROFILE_TRANSPORT=baseline|gzip|http2|both|default` を組み合わせると、圧縮・HTTP バージョンの条件を切り替えられます。通常実行は常にデフォルト機能と全圧縮形式を利用します。以前の調査用 Cargo feature `profile-gzip` / `profile-http2` は不要になりました。
