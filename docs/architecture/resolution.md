# 依存解決と lockfile

依存の取得と実際のディレクトリ操作に先立ち、tnpm は manifest、workspace、レジストリ情報をもとに配置計画を構築します。

## 解決フロー

```text
package.json / workspaces
  ├─ dependencies, devDependencies, optionalDependencies
  ├─ peerDependencies / peerDependenciesMeta
  └─ root overrides / trustedDependencies
               │
               ▼
src/install/mod.rs ─► sources.rs (file, Git, URL, workspace)
               │
               ▼
         resolver.rs ─► resolver/peers.rs
               │
         配置パスと依存グラフ
               ▼
           lockfile.rs
               │
      tnpm-lock.json (v1)
               │
               ▼
     store / tree / lifecycle
```

- `src/manifest.rs` は `package.json` の読み込みと JSON オブジェクトの検証を担当します。
- `src/spec.rs` と `src/semver.rs` はパッケージ指定と範囲条件を処理します。`src/jsr.rs` は JSR の npm 互換名への変換を担当します。
- `src/workspaces.rs` はルート manifest の patterns をもとに、symlink を辿らず workspace を探索します。
- `src/install/overrides.rs` はルートの `overrides` を適用します。推移依存の logical ancestry を考慮します。
- `src/install/resolver/peers.rs` は peer set の共有・分離、バージョンの再探索、不一致検証を担当します。
- `src/install/platform.rs` は `os` / `cpu` / `libc` / `engines` を検証します。

## lockfile の役割

`tnpm-lock.json` は JSON 形式の独自 lockfile で、`tnpmLockfileVersion: 1` を使用します。解決されたパッケージのバージョン、取得元、integrity、依存・配置情報などを保存します。

| 操作 | lockfile の扱い |
| --- | --- |
| `install` | 互換な lockfile があれば再利用し、必要に応じて再解決・保存 |
| `ci` | manifest との整合性を検証してから再配置。lockfile を更新しない |
| `update` | 条件に従い依存を更新して保存 |
| `update --latest` | 直接依存の manifest 宣言も更新 |
| `--no-save` | manifest / lockfile の保存を抑止 |
| `--package-lock-only` | 配置を伴わず解決・保存する。**npm package-lock.json ではない** |

ロック済み依存の到達性・分類には `src/lockfile/graph.rs` の読み取り専用グラフを使います。JSON の文字列を借用し、整数 ID と連続配列で依存先・オフセット・フラグを表すことで、不必要な文字列複製を避けます。

## peer と optional

既定では必須 peer を自動的に解決し、互換な peer set は再利用します。peer の不一致は通常警告ですが、`--strict-peer-deps` では `ERESOLVE` になります。`--legacy-peer-deps` は peer 解決を止めます。

optional 依存はプラットフォーム不適合等で省略でき、`--omit` は**lockfile のレコード削除ではなく物理配置の省略**です。

## 失敗と整合性

- `ci` では lockfile と manifest の不一致を検出してから既存 `node_modules` を削除します。
- lockfile のパスや依存実体の名前・バージョンを検証します。
- `node_modules/.tnpm-lock.json`（hidden lockfile）は既存配置の検査・no-op 最適化に使います。プロジェクトの `tnpm-lock.json` とは役割が異なります。
- 配置前に staging と integrity 検証を行いますが、**全インストール操作はトランザクションではありません**。`ci` の削除や lifecycle 失敗後には一部変更が残り得ます。

[lockfile と install の詳細](../install-and-lockfile.md) / [peerDependencies](../peer-dependencies.md) / [依存ソース](../dependency-sources.md)
