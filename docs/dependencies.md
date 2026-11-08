# 依存ライブラリとビルド構成

性能・サイズ・動作特性に関係する主要な依存関係をまとめます。通常の利用方法は [README](../README.md) を参照してください。

## 依存 feature

- `mimalloc`: tnpm 全体のグローバルアロケータとして使用します。標準構成（mimalloc v3）で、`secure` / `v2` などの追加 feature は有効にしません。ビルドには C コンパイラが必要です。
- `colored`: 進捗、警告・エラー、表、結果メッセージ、末尾のビルド情報を装飾します。stdout / stderr ごとに端末判定し、JSON や `--plain` の表には色を混ぜません。
- `anyhow`: アプリケーションのエラーを統一し、処理の文脈と元の原因を保持します。子プロセスの終了コードは引き続き維持します。
- `phf`（`macros`）: コマンドのエイリアスと npmrc の固定キーを、コンパイル時に生成する Map / Set で検索します。ヘルプの表示順や、利用者が変更する設定の格納方法は維持します。
- `simdutf8`（標準 feature）: レジストリ応答の UTF-8 を `compat` API で検証し、本文全体を String へコピーせず `nojson` に渡します。バイナリキャッシュの共有文字列プールは `basic` API で一度だけ検証します。対応 CPU の SIMD をライブラリ側で選択し、CPU 固有のビルドフラグは強制しません。
- `xxhash-rust`（`xxh3` のみ）: レジストリキャッシュの偶発的な破損を XXH3-128 で検出します。認証用途ではなく、tarball の暗号学的な SRI 検証は変更しません。追加の CPU 固有ビルドフラグは強制しません。
- `reqwest`: デフォルト feature（HTTPS/rustls、HTTP/2、charset、system-proxy）に加え、`gzip`・`brotli`・`deflate`・`zstd` を有効化。対応形式を `Accept-Encoding` で送信し、レスポンスを自動展開します。`json` / `blocking` は有効にせず、JSON は引き続き `nojson` で処理します。
- `tokio`: `default-features = false`、直接指定は `rt` / `time`。Web ログインの完了待ちにタイマーを使用します。通信に必要なネットワーク feature は reqwest 経由で有効になります。シングルスレッド async runtime と展開用の blocking pool を使用します。
- `dirs`: `home_dir()` で `~/.tnpm/cache` の基準となるホームディレクトリを取得します。
- `httpdate`: HTTP の `Date` / `Expires` / `Last-Modified` を解析します（追加の依存先なし）。
- `futures-util`（`std` のみ）: 最大16件の並列メタデータ取得・tarball ダウンロード。
- `tar` / `flate2`: Rust 内で tar/gzip 展開と公開用 tarball の生成。`sha2` / `sha1` / `base64`: SRI integrity 検証、キャッシュキー生成、公開メタデータ。
- `ignore`: pack / publish の `files` と `.npmignore` / `.gitignore` パターンの照合。
- `rpassword`: legacy ログインの端末上のパスワード入力を非表示にします。
- `libc`: Linux の `FICLONE` ioctl。macOS は `clonefile` を直接呼び出し、どちらも clone 非対応時は通常コピーにフォールバックします。
- `unicode-width`（デフォルト feature なし）: `outdated` と `update` の表を文字列の表示幅で揃えます。
