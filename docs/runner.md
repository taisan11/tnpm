# ランナー

`tnpm run` は、`package.json` に定義された名前付きスクリプト、または JavaScript / TypeScript のエントリーファイルを実行します。スクリプト名とファイル名が一致する場合は、常にスクリプトを優先します。

```sh
tnpm run build -- --watch
tnpm run ./src/main.ts -- --port 3000
tnpm run ./src/main.js
```

## ファイルの直接実行

Node.js 22.15 以降（Node 23 系は 23.5 以降）をインストールしてください。Oxc の Rust 製変換器・リゾルバと CommonJS 変換は tnpm に組み込まれており、npm の loader パッケージは不要です。tnpm は独自の同期モジュールフックを付けて Node を起動し、変換や未解決の import にはブリッジを呼び出し、実行自体は Node に任せます。TypeScript は変換のみで、型検査は行いません。必要に応じて別の型検査器を実行してください。

エントリーの拡張子は `.js`、`.jsx`、`.mjs`、`.cjs`、`.ts`、`.tsx`、`.mts`、`.cts` のいずれかである必要があります。まず Node の通常のモジュール解決を試し、見つからない import を Oxc で解決します。対応する `tsconfig.json` のパスエイリアスや TypeScript の拡張子も対象です。変換には Oxc の既定オプションを適用し、CommonJS ファイルの import/export 構文は Oxc の AST を使って変換します。tsconfig の変換オプションとソースマップは未対応です。CommonJS 変換は名前付き・デフォルト import のライブバインディング、再 export、名前空間オブジェクトを維持します。トップレベルの `this` と `import.meta` は書き換えません。変換が必要なソースファイルには、初回使用時に起動する1つの常駐ブリッジプロセスを使い、標準入出力でフレーム化した要求を交換します。変換・解決・リモート取得の結果は実行中メモ化します。利用可能なパイプ記述子がないプラットフォームでは、要求ごとに別の tnpm プロセスを起動する方式へフォールバックします。フックはメインの Node プロセスに登録し、worker スレッドや独立して起動した Node プロセスには自動継承しません。

エントリーのパスは実行時のディレクトリを基準とし、`--prefix` 指定時はそのディレクトリを基準とします。直接実行に package.json は不要です。エントリー以降の引数はシェルで解釈せず Node に直接渡します。子プロセスは標準入力・標準出力・標準エラーを継承し、通常の終了コードを引き継ぎます。エントリーの実行では npm lifecycle フックを実行しません。エントリー前の Node フラグや watch モードは現在未対応のため、必要な場合は Node を直接使ってください。

## JSR・npm・URL import

直接実行では `jsr:@scope/package@range[/export]`、`npm:package@range[/export]`、HTTP(S) import も解決します。JSR は公式の npm 互換配布物を使います。条件に合うローカルのインストール済みパッケージを優先し、なければ lifecycle フックを無効にしてランナー専用の隔離キャッシュへインストールします。HTTP モジュールは最終 URL を保持し、相対・動的 import の解決と TypeScript の変換を行います。JSON import には `with { type: 'json' }` が必要です。Node の既存のファイル・組み込みモジュール・data URL の処理は維持します。

キャッシュだけで実行する場合は `tnpm run --offline FILE`、リモートモジュールを再取得する場合は `tnpm run --no-cache FILE` を使います。URL キャッシュはプロジェクトの lockfile に固定しません。リモート CommonJS、URL エントリー、Deno API、import map は未対応です。これらの機能は import したコードをサンドボックスで隔離しません。キャッシュ・認証・プロトコル境界・制限は [JSR と URL import](jsr-and-url-imports.md) を参照してください。

## パッケージスクリプト

引数なしの `tnpm run` はスクリプト一覧を表示します（`--json` で JSON 出力）。名前付きスクリプトは、ローカルの `node_modules/.bin` ディレクトリを PATH に追加し、プロジェクトルートでシステムのシェルを使って実行します。引数はメインのスクリプトに追加し、フックには渡しません。`--ignore-scripts` を指定しなければ、`pre<name>` をメインのスクリプトの前に、`post<name>` をメインのスクリプトが成功した後に実行します。`--if-present` は指定したスクリプト名が存在しない場合のエラーを抑制します。ファイルを直接指定した場合は、ファイルがなければ引き続きエラーです。

lifecycle の環境変数には `npm_lifecycle_event`、`npm_lifecycle_script`、`npm_package_json`、`npm_package_name`、`npm_package_version`、`npm_execpath`、`INIT_CWD` が含まれます。スクリプトは記述どおりに実行し、任意のシェルコマンドにランナーを注入しません。
