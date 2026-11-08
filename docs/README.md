# ドキュメント

初めて使う場合は [README](../README.md) と [インストール](installation.md) から参照してください。

## 利用者向け

- [インストール](installation.md): 対応 OS、インストーラー、PATH、手動インストール
- [コマンドリファレンス](commands.md): コマンド、フラグ、出力と更新の挙動
- [CLI リファレンス](cli.md): 全コマンド・オプション、引数の解釈、終了コード
- [対応範囲と制限](compatibility.md): npm 互換範囲、未対応機能
- [信頼設定と依存調査](security-and-diagnostics.md): スクリプト許可リスト、rebuild、why / explain、audit
- [workspaces と依存ソース](dependency-sources.md): ローカル / Git 依存、overrides、互換性検証、ネイティブビルド、グローバル配置
- [CLI の結果表示・進捗・ビルド情報](cli-output.md): update / outdated、進捗カウンター、末尾のバージョン情報
- [.npmrc と npm: エイリアス](npmrc.md): 設定優先順位、環境変数、スコープ別認証と JSR
- [認証と公開](publishing.md): login / logout / whoami、pack / publish / dist-tag、公開ファイルと制限
- [JSR と URL import](jsr-and-url-imports.md): jsr: のインストール、HTTP(S) / jsr: / npm: のランナー解決
- [ランナー](runner.md): JS / TS 直接実行と Node の必要バージョン

## 実装・開発者向け

- [プロジェクト構成](architecture.md): モジュールの責務、依存の境界、変更箇所の目安
- [開発・検証](development.md): ビルド、静的チェック、テスト、fixture の方針
- [依存ライブラリ](dependencies.md): Rust クレートの採用理由とビルド feature
- [インストール・lockfile](install-and-lockfile.md): 依存解決、展開・配置、更新最適化
- [peerDependencies](peer-dependencies.md): 対応仕様と検証範囲
- [レジストリキャッシュ](registry-cache.md): メタデータキャッシュと HTTP キャッシュポリシー
- [バイナリキャッシュ形式](cache-format.md): 永続化形式の詳細
- [ベンチマーク](../benchmarks/README.md): 測定条件、実行方法、結果
- [コーディングエージェント向けガイド](../AGENTS.md): 変更時の規約と検証
