# 対応範囲と既知の制限

npm 完全互換を目的とした実装ではありません。npm の `package-lock.json` は読み書きせず、`tnpm-lock.json` を使用します。プロジェクトに導入する前に制限を確認してください。

## 対応範囲

npm 全体の完全な互換実装ではありません。HTTP(S) tarball URL の直接依存、audit、lockfile のみでの一覧、`fund` のブラウザ選択、npm 内部の全設定既定値は未対応です。

- workspaces はローカルリンクと共有 lockfile、`file:` はディレクトリのコピー / tarball に対応します。workspace パターンは `*` / `?` / `**` と除外パターンに対応し、brace / character class は未対応です。
- Git は Git CLI を使用し、コミット / タグ / ブランチ / `#semver:` を解決してコミットを固定します。submodule とリポジトリ内の `file:` 依存は未対応です。
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
