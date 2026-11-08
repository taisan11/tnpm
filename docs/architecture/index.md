# アーキテクチャ

tnpm は **Rust 2024 edition の単一バイナリ**です。CLI・設定・依存解決・配置・ランナーを別のモジュールに分け、Node.js は JavaScript / TypeScript の実行時に使用します。外部向け Rust ライブラリ API は提供していません。

## 全体の流れ

```text
CLI 引数                         package.json / .npmrc
   │                                      │
   ▼                                      ▼
cli.rs ─────────► app.rs ◄──────── config.rs / manifest.rs
                      │
             ┌────────┼─────────────────────────┐
             ▼        ▼                         ▼
       commands/   install/mod.rs           runner/ + scripts.rs
       (表示・操作)     │                   (JS/TS・プロセス)
                      ▼
                 resolver.rs  ◄───── registry/ (HTTP / metadata)
                      │                        │
                      ▼                        ▼
                lockfile.rs              registry/cache.rs
                      │
                      ▼
                 install/store.rs ─────► 共有キャッシュ
                      │
                      ▼
                 install/tree.rs ──────► node_modules
```

| 領域 | 主なコード | 責務 |
| --- | --- | --- |
| CLI・制御 | `main.rs` / `app.rs` / `cli.rs` | 引数解析、実行分岐、終了コード |
| 設定 | `config.rs` / `config/http.rs` | npmrc・環境変数・CLI 優先順位、認証とレジストリ選択 |
| 依存解決 | `install/resolver.rs` / `resolver/peers.rs` | 依存グラフ、物理配置・peer の決定 |
| lockfile | `lockfile.rs` / `lockfile/graph.rs` | 保存と検証、走査時の読み取り専用グラフ |
| 取得・保存 | `registry/` / `install/store.rs` | metadata、HTTP キャッシュ、tarball 検証・展開 |
| 物理配置 | `install/tree.rs` / `install/isolated.rs` / `install/hardlinks.rs` / `install/junction.rs` | hoisted・isolated と bin / link / lifecycle |
| 実行 | `scripts.rs` / `runner/` | shell script、Node.js module hook、JS/TS 変換 |
| ワークスペース | `workspaces.rs` | package 探索・選択 |
| CLI 補助 | `commands/` / `util/style/` | auth、公開、診断、出力整形 |

## 詳細な設計資料

- [依存解決と lockfile](./resolution.md) — manifest からグラフを作るまで
- [レジストリ・キャッシュ・配置](./storage.md) — キャッシュ、共有ストア、ハードリンクと Junction
- [CLI・ランナー・スクリプト](./runtime.md) — コマンド振り分け、Node との連携
- [全モジュールの対応表](../architecture.md) — ソースファイル単位の責務と変更箇所
- [開発・検証](../development.md) — cargo、fixture、テスト、リリース

## 設計上の主な境界

1. **resolver は配置計画、tree は実ファイル操作。** 通信とファイル変更を混ぜない。
2. **manifest と lockfile は別物。** manifest はユーザー宣言、lockfile は固定した解決結果。`ci` は照合するが書き換えない。
3. **registry と store は別物。** registry はメタデータ取得・HTTP 再検証、store は取得物の検証・展開・再利用を担当。
4. **CLI は npm 全互換ではない。** サポートする範囲だけを明示し、未実装機能を前提にしない。
5. **スクリプト実行は任意コードの実行。** lifecycle の名前ベース許可はサンドボックスではない。

[利用者向けリファレンス](/reference/) と区別し、この章では現在の実装に即した責務とデータの流れを説明します。
