# リファレンス

tnpm が現在サポートする**設定ファイルとコマンドの実装範囲**を、実際の Rust ソースと既存の仕様書に沿って整理しています。npm のドキュメントに記載されたすべての機能をサポートするという意味ではありません。

## 設定

| リファレンス | 説明 |
| --- | --- |
| [package.json](./package-json.md) | 依存宣言、scripts、workspaces、overrides、trustedDependencies、公開用フィールド |
| [.npmrc](./npmrc.md) | 設定キー・既定値・スコープ・優先順位・環境変数 |
| [設定ファイルの詳細仕様](../npmrc.md) | INI の解釈、JSR、レジストリ認証と境界条件 |

## コマンド

[コマンド一覧](./cli/index.md) から用途別に確認できます。

| 分類 | 主なコマンド |
| --- | --- |
| [依存関係の操作](./cli/dependencies.md) | `install`、`add`、`ci`、`uninstall`、`update`、`outdated`、`prune` |
| [実行](./cli/execution.md) | `run`、`exec`、`create`、`init`、workspace scripts |
| [調査・診断](./cli/inspection.md) | `ls`、`view`、`why`、`audit`、`fund` |
| [認証・公開](./cli/publishing.md) | `login`、`whoami`、`pack`、`publish`、`dist-tag` |
| [管理・保守](./cli/maintenance.md) | `get`、`cache`、`upgrade`、`alias`、`version`、`rebuild` |

全フラグ・位置依存の引数解釈は [CLI オプションリファレンス](../cli.md) を参照してください。

## 開発者向け

[アーキテクチャ](/architecture/) は使い方ではなく、Rust モジュール間の責務、データフロー、ストアの扱いをまとめた独立セクションです。実装を変更する場合は [開発・検証](/development) も参照してください。
