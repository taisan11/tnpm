# 対応範囲と既知の制限

npm 完全互換を目的とした実装ではありません。npm の `package-lock.json` は読み書きせず、`tnpm-lock.json` を使用します。プロジェクトに導入する前に制限を確認してください。

## 対応範囲

npm 全体の完全な互換実装ではありません。workspaces、グローバルインストール、file/Git/URL 依存、bundled dependencies、lockfile のみでの一覧、`fund` のブラウザ選択、npm 内部の全設定既定値は未対応です。peer の機能は対応していますが npm Arborist のすべての配置・バージョン選択順との一致を保証するものではありません。os/cpu の optional 除外は対応、libc/engine 検証・node-gyp 自動実行・audit は未対応です。主に macOS/Linux 向けで Windows の bin shim は簡易版です。JSON は通常2スペースで再整形しますが、独自 lockfile のパッケージ行はコンパクトに出力します。`fund --json` は名前でまとめたフラットな一覧です。スクリプトや Git 操作失敗後は、npm と同様に変更が残る場合があります。

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

テストのレジストリ通信にはローカル HTTP サーバーを使い、実際の npm レジストリには接続しません。
