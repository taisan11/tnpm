# tnpm

Rust 製の軽量な npm 互換パッケージマネージャー。**npm との完全互換性よりも速度とシンプルさを優先**して開発しています。

## 特徴

- `package.json` の依存関係を解決し、`node_modules` と `tnpm-lock.json` を生成
- npm の `package-lock.json` v2/v3 の読み込み、`npm:` エイリアス、`peerDependencies` に対応
- 並列ダウンロード、メタデータキャッシュ、展開済みパッケージの再利用でインストールを高速化
- `run`、`exec`、`outdated`、`update`、`view`、`cache` などの CLI コマンドを提供

## インストール

[GitHub Releases](https://github.com/taisan11/tnpm/releases) の最新安定版をインストールします。**取得した実行ファイルは公開されている `SHA256SUMS` で検証**します。

### Windows（PowerShell、x64）

```powershell
irm https://raw.githubusercontent.com/taisan11/tnpm/main/install.ps1 | iex
```

ユーザー専用の `%LOCALAPPDATA%\Programs\tnpm` にインストールし、**ユーザー環境変数の PATH に自動登録**します。管理者権限は不要です。

### Linux / macOS

```sh
curl -fsSL https://raw.githubusercontent.com/taisan11/tnpm/main/install.sh | sh
```

`~/.local/bin/tnpm` にインストールします。ディレクトリが PATH に含まれていない場合は、利用中のシェルの設定に追加してください。`curl` の代わりに `wget` でも実行できます（スクリプトを保存して `sh install.sh`）。

公式ビルドは Linux x86_64 / ARM64（glibc 2.35 以上）、macOS Intel / Apple Silicon、Windows x64 に対応します。インストール先の変更、手動実行、ソースからのビルドは [インストールガイド](docs/installation.md) を参照してください。

## 使い始める

```sh
tnpm init -y                 # package.json の作成
tnpm install                 # 既存プロジェクトの依存をインストール
tnpm add react               # 依存パッケージの追加
tnpm add typescript -D       # 開発用依存として追加
tnpm run build               # package.json のスクリプトを実行
tnpm x tsc --version         # ローカル bin を実行（なければ取得）
tnpm outdated                # 更新可能な直接依存を表示
tnpm update                  # 宣言されたバージョン範囲内で更新
tnpm upgrade                 # tnpm 本体を最新安定版に更新
```

詳しいコマンド、エイリアス、オプションは [コマンドリファレンス](docs/commands.md) または `tnpm --help` を参照してください。

## npm との互換性

npm の一部コマンドと lockfile をサポートしていますが、**完全なドロップイン置換ではありません**。workspaces、グローバルインストール、file/Git/URL 依存、audit などには未対応です。詳細は [対応範囲と制限](docs/compatibility.md) を確認してください。

## ドキュメント・開発

- [ドキュメント一覧](docs/README.md)
- [インストール方法](docs/installation.md) / [コマンド一覧](docs/commands.md)
- [依存解決・lockfile](docs/install-and-lockfile.md) / [キャッシュ](docs/registry-cache.md)
- [アーキテクチャ](docs/architecture.md) / [開発とテスト](docs/development.md)
- [ベンチマーク](benchmarks/README.md) / [コーディングエージェント向けガイド](AGENTS.md)

ソースからのビルドには Rust 2024 edition 対応のツールチェーンが必要です。

```sh
cargo build --release
./target/release/tnpm --help
```
