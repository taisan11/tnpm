# 対応範囲と既知の制限

npm 完全互換を目的とした実装ではありません。npm の `package-lock.json` は読み書きせず、`tnpm-lock.json` を使用します。プロジェクトに導入する前に制限を確認してください。

## 対応範囲

- 認証系 `login` / `adduser` / `logout` / `whoami`、ローカルディレクトリの `pack` / `publish`、`dist-tag` に対応します。`unpublish`、provenance / OIDC、bundled dependencies の梱包、workspace 一括公開、tarball からの公開は未対応です。公開ファイル選択は npm より保守的で、秘密ファイル・symlink・lockfile を除外します。[認証と公開](publishing.md)。

npm 全体の完全な互換実装ではありません。`audit fix`、Quick Audit へのフォールバック、meta-vulnerability の算出、`ls` の lockfile のみでの一覧、`fund` のブラウザ選択、npm 内部の全設定既定値は未対応です。

- workspaces はローカルリンクと共有 lockfile、`file:` はディレクトリのコピー / tarball に対応します。workspace パターンは Rust の `glob` crate で `*` / `?` / `**` / 文字クラス（`[ab]`, `[0-9]` など）と先頭 `!` の除外に対応します。brace 展開（`{a,b}`）は未対応です。
- Git は Git CLI を使用し、コミット / タグ / ブランチ / `#semver:` を解決してコミットを固定します。submodule とリポジトリ内の `file:` 依存は未対応です。
- HTTP(S) tarball URL は直接・推移依存と CLI の裸 URL / `name@URL` に対応します。SHA-512 を固定し、URL の認証情報・archive 内のリンクや traversal・リモート archive 内の `file:` 依存を拒否します。
- `jsr:` は公式の npm 互換配布物を使い、@jsr スコープの既定レジストリは npm.jsr.io です。直接実行のランナーは HTTP(S) / jsr: / npm: import を解決します。Deno API・import map・URL エントリー実行は未対応です。[JSR と URL import](jsr-and-url-imports.md)。
- 依存の lifecycle / 暗黙の node-gyp は許可リスト制です。Bun 由来の組み込みリストにルートの `trustedDependencies` と `--trust NAME` を**追加**します（置き換えません）。ルート / workspace 自身は許可します。`rebuild` にも同じ制約が適用されます。[信頼設定と依存調査](security-and-diagnostics.md)。
- `why` / `explain` は tnpm lockfile の依存経路と直接の要求元を表示します。インストール済みツリーとの一致は保証しません。`audit` はロックされたレジストリ依存を各設定レジストリの Bulk Advisory API で検査します。
- root の `overrides`（文字列・ネスト・`.`・`$` 参照・バージョン条件）を解決します。bundled dependency 自体を別バージョンへ差し替える override は拒否します。
- os / cpu / Linux libc の検証と optional 除外、`engines.node` / `engines.tnpm` の警告と `--engine-strict` に対応します。`engines.npm` は検証しません。
- `binding.gyp` に対する既定の `node-gyp rebuild` と tarball 内の bundled dependencies に対応します。node-gyp / Python / コンパイラーの自動配布は行いません。
- `install` / `uninstall` / `update` / `outdated` / `ls` の `-g` に対応します。既定 prefix は npm と異なる `~/.tnpm/global` です。PATH は変更しません。

詳細は [workspaces と依存ソース](dependency-sources.md) を参照してください。peer の機能は対応していますが npm Arborist のすべての配置・バージョン選択順との一致を保証するものではありません。主に macOS/Linux 向けで Windows の bin shim は簡易版です。Windows の workspace リンクには Developer Mode などの symlink 作成権限が必要です。JSON は通常2スペースで再整形しますが、独自 lockfile のパッケージ行はコンパクトに出力します。`fund --json` は名前でまとめたフラットな一覧です。スクリプトや Git 操作失敗後は、npm と同様に変更が残る場合があります。

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

テストのレジストリ通信にはローカル HTTP サーバーを使い、実際の npm レジストリには接続しません。
