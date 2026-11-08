---
layout: home

hero:
  name: tnpm
  text: 高速でシンプルなパッケージ管理
  tagline: Rust 製 npm 互換パッケージマネージャーのガイドとリファレンス
  actions:
    - theme: brand
      text: インストールする
      link: /installation
    - theme: alt
      text: コマンドを調べる
      link: /reference/cli/
    - theme: alt
      text: 内部構造を見る
      link: /architecture/

features:
  - title: npm 互換の基本操作
    details: install・ci・run・exec・公開系コマンドと、対応するオプションを検索できます。
    link: /reference/cli/
  - title: 設定ファイルのリファレンス
    details: package.json と .npmrc の読み取り範囲、優先順位、設定例、非対応事項を明記。
    link: /reference/
  - title: 分離配置と共有ストア
    details: hoisted / isolated の仕組み、ファイルハードリンクと Windows Junction の制約を説明。
    link: /architecture/storage
  - title: 実装を追える設計資料
    details: CLI、依存解決、レジストリ、lockfile、ランナーの責務を独立した章に整理。
    link: /architecture/
---

## クイックスタート

```sh
tnpm init -y
tnpm add react
tnpm add typescript -D
tnpm run build
```

tnpm は `tnpm-lock.json` を使用し、npm の `package-lock.json` は読み書きしません。依存のライフサイクルスクリプトも既定では許可リスト制です。移行前に [対応範囲と制限](/compatibility) を確認してください。

### 目的から探す

- **使い始める** → [インストール](/installation)、[コマンド一覧](/reference/cli/)
- **設定する** → [package.json](/reference/package-json)、[.npmrc](/reference/npmrc)
- **仕組みを調べる** → [アーキテクチャ](/architecture/)、[依存解決](/architecture/resolution)
- **既存の詳細仕様** → [ドキュメント一覧](/README)
