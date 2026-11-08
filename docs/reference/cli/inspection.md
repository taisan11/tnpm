# 調査と診断

## `ls`

```sh
tnpm ls
tnpm ls --all
tnpm ls --depth=1 --json
tnpm ls react
tnpm ls -g
```

`node_modules` にある依存を表示します。既定は直接依存のみで、`--all` / `-a` は推移依存まで表示。`--depth=N` は深さを制限します。必須依存の欠落・不一致は終了コード `1` です。

## `view`

```sh
tnpm view react
tnpm view react version
tnpm view 'react@^19' dist.tarball --json
tnpm view jsr:@luca/cases version
```

レジストリのメタデータを参照します。既定は人間向け整形表示で、`--json` は JSON を返します。バージョン / タグ / 範囲に対応し、フィールド選択ではドット区切りパスと配列添字を使用できます。パッケージ省略または `.` の場合はローカル manifest の名前を利用します。

## `explain` / `why` {#explain}

```sh
tnpm why react
tnpm explain 'react@^19' --json
tnpm explain node_modules/parent/node_modules/react
```

`tnpm-lock.json` の依存グラフから要求元・範囲・依存種別と、ルートからの最短経路の一例を示します。**実際の node_modules のスキャンではありません。** クエリが存在しない場合はエラーです。

## `audit`

```sh
tnpm audit
tnpm audit --omit=dev --audit-level=high
tnpm audit --json --audit-level=none
```

lockfile にあるレジストリ依存を対象レジストリの Bulk Advisory API に照会します。`node_modules` は必要ありません。

| オプション | 意味 |
| --- | --- |
| `--audit-level=info\|low\|moderate\|high\|critical\|none` | 終了コードを失敗にする重大度の閾値。表示のフィルタではない |
| `--omit=dev,optional,peer` | 対象外にする依存種別 |
| `--json` | advisory・重大度集計・除外項目を JSON 出力 |

自動修正機能（`audit fix`）はありません。`--offline` は拒否します。local / Git / URL tarball / workspace 自体は対象外です。

## `fund`

```sh
tnpm fund
tnpm fund --json
```

インストール済み依存の `funding` URL を列挙します。ブラウザを自動で開くことはなく、パッケージ選択引数は未対応です。

[信頼設定と診断仕様](../../security-and-diagnostics.md) / [出力仕様](../../cli-output.md)
