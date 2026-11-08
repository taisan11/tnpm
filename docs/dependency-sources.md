# workspaces と依存ソース

## workspaces

ルートの `package.json` に workspace のディレクトリを宣言します。

```json
{
  "private": true,
  "workspaces": ["packages/*"],
  "overrides": { "some-transitive-package": "^2.0.0" }
}
```

`tnpm install` は各 workspace を名前で `node_modules` にリンクし、通常依存・開発依存・optional・peer を共有グラフで解決します。競合するバージョンは workspace 内の `node_modules` に配置します。開発依存は `--omit=dev` で省略でき、ルートの `tnpm-lock.json` で一括管理します。`ci` はルートだけでなく workspace の node_modules も削除・再構築します。workspace 間は通常の一致する SemVer、または `workspace:*` / `workspace:^` / `workspace:~` / `workspace:^1.0.0` で参照できます。

```sh
tnpm install
tnpm add typescript -D -w my-package
tnpm uninstall typescript -w my-package
tnpm run -w my-package build
tnpm run --workspaces --if-present test
tnpm run --workspaces --parallel test
tnpm run --workspaces --topological --concurrency 4 build
tnpm ci --ignore-scripts
```

`-w` / `--workspace` は名前・相対ディレクトリ・親ディレクトリで指定でき、複数回指定できます。`--workspaces` は全 workspace が対象です。スクリプト名以降はスクリプトへ転送されるため、選択オプションはスクリプト名より前に指定してください。workspace 内からの `run`、依存追加・削除、`update --latest` はその workspace を暗黙的に選択します。install / ci は選択にかかわらず共有グラフ全体を構築します。通常の update も共有グラフを更新します。

パターンは `*` / `?` / `**` と `!` による除外を扱います。隠しディレクトリ、node_modules、ディレクトリ symlink は探索しません。同名 workspace、不一致のルート直接依存、プロジェクト外へのリンクはエラーです。既存リンクの参照先が宣言と違う場合も拒否します。Windows では Developer Mode など symlink 作成権限が必要です。

### スクリプトの並列・依存順実行

`run` の `--parallel` は利用可能 CPU 数を上限に選択 workspace を並列実行し、`--concurrency N` は上限を指定して並列実行を有効にします。`--topological` は選択内の通常・開発・optional 依存を、範囲が一致する workspace の依存先から実行します。peer は順序制約にしません。`--topological` 単独は直列、`--parallel` / `--concurrency` との併用では同じ依存段階を並列に実行します。

すべてスクリプト名より前に指定してください。選択外の依存 workspace は自動追加しません。循環は実行開始前にエラーにします。失敗時は開始済みの同じバッチを待ち、次のバッチ・依存段階を起動せず子の終了コードを返します。引数転送・pre/post・`--if-present` は通常の run と同じです。並列時の標準出力・標準エラーは混在し得ます。これらのフラグを付けない場合は従来の選択順の逐次実行です。

## HTTP(S) tarball URL

```sh
tnpm add https://example.com/helper-1.0.0.tgz
tnpm add 'helper@https://example.com/helper.tgz'
```

manifest の直接・推移依存にも URL を指定できます。初回は安全な一時ディレクトリで archive の manifest を読み、裸 URL ならその名前を採用します。lockfile に元 URL と SHA-512 integrity を保存し、通常 install / ci では固定した内容を使います。`update` で再取得します。`outdated` / `update --latest` のレジストリ対象からは除外します。

既存の認証・TLS・リダイレクト境界、tar traversal / symlink / 特殊ファイル拒否と配置前の integrity 検証を使います。userinfo を持つ URL は拒否し、認証は npmrc の URL スコープ設定を使います。リモート archive 内の `file:` 依存は未対応です。

非認証時は検証済み元 archive を URL と integrity のハッシュキーで `~/.tnpm/cache/source-tarballs-v1/` に保存し、展開済みキャッシュとともに固定済み依存の `ci --offline` に利用できます。初回の URL 解決には通信が必要です。認証付き・`--no-cache` では永続キャッシュを読み書きしません。

## ローカル依存

```sh
tnpm add ./packages/helper
tnpm add ./helper.tgz
tnpm add 'helper@file:../helper'
```

`file:` ディレクトリは **リンクではなくコピー** です。`.git` / `node_modules` / tnpm の staging は含めません。再 install で内容を更新します。依存内の相対 `file:` は元のディレクトリを基準に解決します。コピー中の symlink・特殊ファイルは拒否します。

`outdated` は `file:` / Git / `workspace:` 指定をレジストリの更新確認から除外します。`update --latest` は従来どおりレジストリ依存専用です。これらのソースを含む場合はレジストリの対象名を明示するか、通常の install / update を使ってください。

ローカル tarball は SHA-512 を lockfile に保存します。`ci` はローカル manifest の依存・バージョン・互換性条件と lockfile、tarball の integrity を照合し、不一致なら既存 node_modules の削除前に失敗します。ディレクトリ内容全体のハッシュは固定しません。相対パスを使用すると同じ配置の別 checkout でも利用できます。

## Git 依存

```json
{
  "dependencies": {
    "helper": "git+https://example.com/team/helper.git#v1.0.0",
    "another": "github:team/another#semver:^2.0.0"
  }
}
```

`git+https:` / `git+ssh:` / `git://` / `git@host:path` / `git+file:`、GitHub / GitLab / Bitbucket の hosted 指定、GitHub の `owner/repo` 省略形を扱います。省略時は HEAD、`#` 以降はコミット・タグ・ブランチ・`semver:` 範囲です。解決済みコミットと archive の SHA-512 を保存し、通常 install / ci はブランチが動いても固定結果を再利用します。`update` で再解決します。

Git CLI が PATH 上に必要です。checkout や repository hooks は実行せず、`git archive` を既存の安全な tar 展開で配置します。submodule と symlink は含められません。`file:` のリポジトリ内参照は未対応です。認証には SSH / credential helper を使用し、パスワードや HTTPS userinfo を含む URL は拒否します。`~/.tnpm/cache/git-v1` のコミット別 archive があれば `ci --offline` できます。`--no-cache` ではこのキャッシュを使いません。

`prepare` がある Git パッケージでは devDependencies も解決し、許可リストに含まれる場合に配置後の prepare を実行します。npm の隔離ビルド・pack と異なり、これらの開発依存も解決グラフに残ります。許可済みを含めすべて止めるには `--ignore-scripts` を指定してください。

## overrides

ルートの `package.json` のみを参照します。パッケージ自身や workspace 内の overrides は使用しません。

```json
{
  "dependencies": { "shared": "^2.0.0", "parent": "^1.0.0" },
  "overrides": {
    "shared": "$shared",
    "parent@^1": {
      ".": "^1.0.0",
      "child": "^3.0.0"
    }
  }
}
```

文字列の全体 override、ネストした祖先条件、`.` による自身の指定、バージョン条件付きキー、`$name` による直接依存の spec 参照に対応します。ネスト条件は巻き上げ後も論理的な祖先に維持します。直接依存に異なる spec を強制する指定や存在しない `$` 参照は `EOVERRIDE` です。bundled dependency 自体のバージョン差し替えは未対応です。overrides を変更した場合は install で lockfile を更新してください。

## engines / libc とネイティブビルド

- `engines.node` と `engines.tnpm` は不一致時に `EBADENGINE` 警告を出します。Node.js のバージョンは `node --version` で実行単位に一度調べます。
- `--engine-strict` または `.npmrc` の `engine-strict=true` でエラーにします。`--force` は engine-strict を警告に戻します。`engines.npm` は tnpm のバージョンと同一視せず検証しません。
- `os` / `cpu` と Linux の `libc`（glibc / musl）は文字列・配列・否定条件に対応します。不適合な必須依存は `EBADPLATFORM`、optional は配置から除外します。
- `binding.gyp` があり、独自の install / preinstall がなく、`gypfile:false` でない場合、既定の install として `node-gyp rebuild` を実行します。`--ignore-scripts` では実行しません。
- node-gyp は PATH 上のものを使います。`npm_config_node_gyp` で node-gyp の JS エントリーも指定できます。Python / C/C++ コンパイラーなどは別途用意してください。
- tarball 内の `bundleDependencies` / `bundledDependencies` は子の manifest・物理配置を lockfile に保存します。子も bin・lifecycle・optional / platform の処理対象です。tar の traversal / symlink / device 拒否は維持します。

## グローバルインストール

```sh
tnpm install -g typescript
tnpm ls -g
tnpm update -g typescript
tnpm uninstall -g typescript
tnpm install -g --prefix "$HOME/tools" typescript
```

既定 prefix は `~/.tnpm/global` です。CLI の `--prefix`、`npm_config_prefix`、npmrc の `prefix` で変更できます。

| OS | パッケージ | 実行ファイル |
| --- | --- | --- |
| macOS / Linux | `<prefix>/lib/node_modules` | `<prefix>/bin` の symlink |
| Windows | `<prefix>/node_modules` | `<prefix>` の `.cmd` shim |

管理者権限は不要ですが、実行ファイルディレクトリを PATH に追加する必要があります。ユーザー / システム PATH は自動変更しません。グローバル管理用の manifest・lockfile をパッケージ配置先の親に保存します。既存の無関係な bin は上書きせず、削除時も所有記録と内容が一致するものだけを削除します。bin の衝突など後半の処理で失敗した場合、パッケージや管理用 manifest の変更が残ることがあります。

`-g` は install / uninstall / update / outdated / ls に対応し、workspace 指定とは併用できません。所有管理に manifest・lockfile が必要なため `--no-save` は拒否します。管理外のパッケージが既にある prefix や無関係な package.json は変更せずエラーにします。既存の npm prefix ではなく専用 prefix を使ってください。
