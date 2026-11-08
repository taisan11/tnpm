# レジストリ・キャッシュ・配置

`registry/` がメタデータと HTTP を、`install/store.rs` が取得物の検証・展開を、`install/tree.rs` と `install/isolated.rs` が node_modules の生成を担当します。

## レジストリと取得

```text
registry query ──► config/http.rs (scope/auth/TLS)
       │
       ▼
registry/mod.rs ──► HTTP response
       │
       ▼
registry/cache.rs + http_cache.rs (v3 metadata cache)
       │
       ▼
resolver ──► tarball URL / integrity
                  │
                  ▼
            install/store.rs
             ├─ fetch / verify
             ├─ safe extraction
             └─ packages-v2/ cache
```

- メタデータキャッシュは `~/.tnpm/cache/v3/`。SemVer インデックスなどを備え、必要な版・フィールドだけ復元します。
- HTTP は `Cache-Control`、`Expires`、`ETag` 等で鮮度と再検証を判定します。短い TTL に独自の最低5分ポリシーがありますが、`no-store` 等を無視するものではありません。
- 元 tarball の integrity を SHA-512 / SHA-256 / SHA-1 で検証し、tar 内の path traversal・symlink・特殊ファイルを拒否します。
- 認証付き通信結果や `--no-cache` の取得物は共有の永続キャッシュに保存しません。
- 取得済み内容からの再配置に使う `packages-v2/` は integrity に紐づきます。

詳細は [レジストリキャッシュ](../registry-cache.md) と [バイナリ形式](../cache-format.md)。

## hoisted 配置（既定）

`node-linker=hoisted` では一般的な `node_modules` 構造へ依存を巻き上げ、競合はネストします。展開済みパッケージを配置するとき、macOS の clonefile・Linux の reflink を可能な範囲で利用し、非対応なら通常コピーします。

```text
project/
  package.json
  tnpm-lock.json
  node_modules/
    package-a/
    package-b/
    .bin/
    .tnpm-lock.json
```

## isolated 配置とファイルストア

`node-linker=isolated` では `node_modules/.tnpm/<id>/node_modules/<package>` に各パッケージ実体を置き、プロジェクトの直接依存と個々の宣言依存を**ディレクトリリンク**で公開します。

```text
~/.tnpm/cache/                   (store-dir)
  packages-v2/                   検証済み展開キャッシュ
  files-v1/                      SHA-256 内容アドレスのファイル
project/
  node_modules/
    .tnpm/
      <id>/node_modules/package-a/    files-v1/ へのファイル hardlink
    package-a  ──(directory link)──► .tnpm/.../package-a
```

- パッケージのファイルは `fs::hard_link` で共有し、**通常コピー・CoW にはフォールバックしません**。異なるディスク／ボリュームでは失敗します。
- 依存間のディレクトリリンクは Linux/macOS で symlink、Windows で管理者権限不要の **NTFS Junction** を使用します。
- インストール時 lifecycle は現在、一時的な hoisted 配置で実行し、実行後の最終ファイル内容を SHA-256 で `files-v1/` に登録してからハードリンクします。
- hardlink は実ファイルを共有します。`node_modules` 内の依存ファイルを**直接書き換えると共有ストアや別プロジェクトにも影響**します。Unix ではストア登録時に書き込み権限を外しますが、Windows は改変防止を保証しません。
- `isolated` で通常の `prune` / `rebuild` は未対応です。切替時や再インストール時には `node_modules` を再構築します。

```ini
# ~/.npmrc
node-linker=isolated
store-dir=~/.tnpm/cache
```

設定は [`.npmrc` リファレンス](/reference/npmrc) を参照してください。`store-dir` を別パスに変更しても、既存の共有ストアは自動移動しません。

## 重要なディレクトリ

| ディレクトリ | 用途 |
| --- | --- |
| `~/.tnpm/cache/v3/` | レジストリ metadata |
| `<store-dir>/packages-v2/` | 検証済み展開パッケージ |
| `<store-dir>/files-v1/` | isolated のハードリンク元ファイル |
| `~/.tnpm/cache/source-tarballs-v1/` | 直接 URL tarball の検証済み原本 |
| `~/.tnpm/cache/git-v1/` | 固定済み Git archive |
| `~/.tnpm/cache/modules-v1/` | URL import のソースキャッシュ |
| `~/.tnpm/exec/` | 一時実行・runner の隔離依存 |

`tnpm cache rm` は固定の `~/.tnpm/cache` と `~/.tnpm/exec` を削除します。**カスタム `store-dir` の追加パスは削除しません**。

[配置方式と限界](../install-and-lockfile.md#配置方式と共有ストア) / [認証・HTTP の境界](../npmrc.md#レジストリと認証)
