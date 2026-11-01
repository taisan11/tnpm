# インストール

tnpm は GitHub Releases で OS / CPU ごとの単体実行ファイルを配布します。インストーラーは **最新の安定版**を取得し、`SHA256SUMS` に記載された SHA-256 を検証してから配置します。npm や Node.js は tnpm 自体のインストールには不要です（`run` などで JavaScript を実行するには別途 Node.js が必要になる場合があります）。

## 対応する配布バイナリ

| OS | アーキテクチャ | Release asset |
| --- | --- | --- |
| Windows | x64 | `tnpm-x86_64-pc-windows-msvc.exe` |
| Linux（glibc 2.35 以上） | x64 | `tnpm-x86_64-unknown-linux-gnu` |
| Linux（glibc 2.35 以上） | ARM64 | `tnpm-aarch64-unknown-linux-gnu` |
| macOS | Intel | `tnpm-x86_64-apple-darwin` |
| macOS | Apple Silicon | `tnpm-aarch64-apple-darwin` |

Alpine Linux などの musl 環境向けバイナリは現時点で配布していません。

## Windows（PowerShell）

```powershell
irm https://raw.githubusercontent.com/taisan11/tnpm/main/install.ps1 | iex
```

既定の配置先は `%LOCALAPPDATA%\Programs\tnpm\tnpm.exe` です。管理者権限なしでユーザーの `PATH` に追加します。現在の PowerShell セッションの `PATH` にも反映し、他の起動済みターミナルでは再起動が必要な場合があります。既存の PATH 項目は保持し、インストール先が登録済みなら二重追加しません。

スクリプトを確認してから実行する場合:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/taisan11/tnpm/main/install.ps1 -OutFile install.ps1
.\install.ps1
```

## Linux / macOS

```sh
curl -fsSL https://raw.githubusercontent.com/taisan11/tnpm/main/install.sh | sh
```

または事前に内容を確認してから実行できます:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/taisan11/tnpm/main/install.sh
sh install.sh
```

既定の配置先は `$HOME/.local/bin/tnpm` です。インストーラーはシェル設定ファイルを勝手に変更しません。`~/.local/bin` が PATH にない場合は `~/.profile` などに次を追加してください。

```sh
export PATH="$HOME/.local/bin:$PATH"
```

スクリプトは `curl` または `wget` と、`sha256sum`・`shasum`・`openssl` のいずれかを利用します。SHA-256 不一致時や実行不能時はインストール前に停止します。

## インストール先の変更

環境変数 `TNPM_INSTALL_DIR` で両方のインストーラーのディレクトリを変更できます。Linux/macOS では絶対パスを指定します。

```sh
TNPM_INSTALL_DIR="$HOME/bin" sh install.sh
```

```powershell
$env:TNPM_INSTALL_DIR = Join-Path $HOME 'bin'
.\install.ps1
```

Windows では変更後のディレクトリをユーザー PATH に追加します（以前のディレクトリは自動削除しません）。Linux/macOS では PATH の調整は利用者が行います。

## 手動インストール・更新

[Releases](https://github.com/taisan11/tnpm/releases) の対応する実行ファイルと `SHA256SUMS` を取得して照合し、Windows では `tnpm.exe`、Unix では `tnpm` にリネームして PATH の通った場所へ配置します。Unix では実行権限が必要です。

`tnpm upgrade` はインストール済みの本体を新しい安定版へ更新できます。既存ファイルの更新にはインストール先への書き込み権限が必要です。

## ソースからビルド

Rust 2024 edition 対応のツールチェーンと C コンパイラが必要です。

```sh
git clone https://github.com/taisan11/tnpm.git
cd tnpm
cargo build --release
```

詳細は [開発・検証](development.md) を参照してください。
