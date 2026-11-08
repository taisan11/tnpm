# 信頼設定と依存調査

## 依存スクリプトの許可リスト

**依存パッケージのスクリプトは既定で全実行ではなく許可リスト制です。** `preinstall` / `install` / `postinstall`、ソース依存の `prepare` と暗黙の `node-gyp rebuild` に適用します。ルートと宣言済み workspace 自身はプロジェクトのコードとして許可します。

許可集合は次の**和集合**です。

1. [Bun の default-trusted-dependencies.txt](https://github.com/oven-sh/bun/blob/5c59842f78880a8b5d9c2eb99c8928fc2ec50a2d/src/install/default-trusted-dependencies.txt) から取り込んだ組み込みリスト。
2. ルート `package.json` の `trustedDependencies` 配列。
3. コマンドラインの `--trust PACKAGE`（複数回指定可）。

```json
{
  "dependencies": { "my-native": "^1.0.0" },
  "trustedDependencies": ["my-native"]
}
```

この例でも esbuild / sharp などの組み込み許可は残ります。空配列も既定リストを無効にしません。組み込みリストはバイナリに同梱し、実行時に GitHub へ取りに行きません。取り込み元とライセンスは [third-party/Bun.md](https://github.com/taisan11/tnpm/blob/main/third-party/Bun.md) に記録しています。

- 許可する名前はパッケージの**実名**です。`npm:` alias の配置名を信頼済みの名前にしても許可されません。ロックされた実名と実際の manifest の名前の一致も検査します。
- 依存パッケージ自身や workspace 内の `trustedDependencies` は読みません。外部パッケージが自己承認することはできません。
- ブロック時は stderr に対象と許可方法を表示し、インストール自体は継続します。
- `--trust` はその実行だけの追加で、manifest を編集しません。`install` / `ci` / `update` / `uninstall` / `exec` / `create` / `rebuild` の依存処理に使用します。
- `--ignore-scripts` または npmrc の `ignore-scripts=true` は、許可済み・ルート・workspace を含め install 時のフックを止めます。明示した `run SCRIPT` は従来どおり実行します。
- 後から許可した既存依存は、次回 `install` でも未実行のフックを実行します。成功記録は hidden lockfile 内の `tnpmBuilds` に保存し、通常の no-op install では繰り返しません。`ci` はクリーンに再実行します。

これは**名前に基づく実行許可であり、サンドボックスやパッケージの安全性保証ではありません**。許可したスクリプトは通常のユーザー権限で動き、ネットワーク・ファイルなどへアクセスできます。組み込み許可もすべて止めたい場合は `--ignore-scripts` を使ってください。

## rebuild

```sh
tnpm rebuild
tnpm rebuild esbuild
tnpm rebuild 'my-native@^1' --trust my-native
tnpm rebuild node_modules/parent/node_modules/child --json
```

インストール済みの依存を対象に bin を再生成し、許可された依存の preinstall / install / postinstall、ソース依存の prepare、暗黙の node-gyp を実行します。全体指定または複数の名前・SemVer 範囲・プロジェクト相対 node_modules パスに対応します。ネストの深い依存から処理し、依存先グラフ全体のトポロジカル順は保証しません。

通信・再解決・パッケージ取得・`package.json` / `tnpm-lock.json` の書き換えはしません。`--offline` も利用できます（実行するスクリプト自身の通信は制限しません）。`--ignore-scripts` では bin の再生成だけを行います。node-gyp / Python / コンパイラーは別途必要です。指定の未一致や不正な symlink はエラーです。スクリプト失敗の終了コードは引き継ぎます。`--json` は `{ "rebuilt": N }` を stdout に出し、フック出力は stderr に送ります。N は実行対象となった許可済み lifecycle パッケージ数です。

## why / explain

```sh
tnpm why react
tnpm explain 'react@^19' --json
tnpm why node_modules/parent/node_modules/react
tnpm why react @scope/package
```

`tnpm-lock.json` の物理配置と依存関係を読み、次を表示します。

- 該当パッケージの実名・バージョン・配置パス。
- ルートからの最短経路の一例。
- 到達可能な全直接要求元と、要求範囲・依存種別（prod / dev / optional / peer / peerOptional）。

巻き上げ、ネスト、alias、peer、循環を扱います。`node_modules` とネットワークは不要ですが、インストール済みツリーを走査するコマンドではありません。手動変更や `--no-save` 後の状態とは異なる場合があります。`--json` は `name` / `version` / `location` / `chain` / `dependents` を持つ配列です。指定が1つでも未一致なら出力前に失敗します。

## audit

```sh
tnpm audit
tnpm audit --omit=dev --audit-level=high
tnpm audit --json --audit-level=none
```

整合する manifest と `tnpm-lock.json` が必要です。`node_modules` は不要です。ロックされたレジストリパッケージの実名と重複排除したバージョン一覧を、各設定レジストリの `/-/npm/v1/security/advisories/bulk` へ POST します。スコープ別レジストリ・認証・TLS 設定と registry URL のサブパスを尊重します。明示的な `--registry` は従来どおりスコープ設定より優先します。インストール時に自動実行することはありません。

- `--omit=dev,optional,peer` / `--production` と npmrc の omit 設定を反映し、除外依存からしか到達できない子も送信しません。
- `file:` / Git / tarball URL / workspace 自体はレジストリ配布物と同一と見なさずスキップします。その下のレジストリ依存は対象です。
- private scope の名前をデフォルトの公開レジストリにまとめて送ることはありません。別 origin へのリダイレクトは認証の有無にかかわらず拒否します。
- advisory の範囲に実際に一致する版と配置箇所だけを表示します。`--audit-level` は表示の絞り込みではなく終了コードの閾値です。info（既定）/ low / moderate / high / critical / none を使え、npmrc の `audit-level` も利用できます。
- `--json` は `advisories`、重大度別 `vulnerabilities` 集計、`skipped` を返します。各 advisory の `affected` に version / location の配列が入ります。件数は**一致した advisory 数**であり、npm の meta-vulnerability を含むパッケージ数とは異なります。
- 毎回オンラインで問い合わせ、結果をキャッシュしません。`--offline`、未対応レジストリ、通信失敗、不正な応答はエラーです。検査できなかった状態を「問題なし」とは扱いません。

`audit fix`、Quick Audit へのフォールバック、meta-vulnerability / 修正候補の計算はありません。依存の変更やスクリプト実行は行いません。結果が0件でも、スキップ対象やデータベースに未登録の脆弱性まで安全と保証するものではありません。
