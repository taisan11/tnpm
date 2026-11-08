import { defineConfig } from 'vitepress'

export default defineConfig({
  lang: 'ja-JP',
  title: 'tnpm',
  description: 'Rust 製パッケージマネージャー tnpm の公式ドキュメント',
  base: process.env.VITEPRESS_BASE || '/tnpm/',
  lastUpdated: true,
  themeConfig: {
    logo: undefined,
    siteTitle: 'tnpm',
    nav: [
      { text: 'ガイド', link: '/installation' },
      { text: 'リファレンス', link: '/reference/' },
      { text: 'アーキテクチャ', link: '/architecture/' },
      { text: 'GitHub', link: 'https://github.com/taisan11/tnpm' },
    ],
    sidebar: {
      '/reference/': [
        { text: 'リファレンス', items: [
          { text: '概要', link: '/reference/' },
          { text: 'package.json', link: '/reference/package-json' },
          { text: '.npmrc', link: '/reference/npmrc' },
        ] },
        { text: 'CLI コマンド', items: [
          { text: 'コマンド一覧', link: '/reference/cli/' },
          { text: '依存関係の操作', link: '/reference/cli/dependencies' },
          { text: 'スクリプトと実行', link: '/reference/cli/execution' },
          { text: '調査と診断', link: '/reference/cli/inspection' },
          { text: '認証と公開', link: '/reference/cli/publishing' },
          { text: '管理と保守', link: '/reference/cli/maintenance' },
          { text: 'オプションと引数', link: '/cli' },
        ] },
        { text: '関連仕様', collapsed: true, items: [
          { text: '配置と lockfile', link: '/install-and-lockfile' },
          { text: '対応範囲', link: '/compatibility' },
          { text: '出力形式', link: '/cli-output' },
        ] },
      ],
      '/architecture/': [
        { text: 'アーキテクチャ', items: [
          { text: '全体像', link: '/architecture/' },
          { text: '依存解決と lockfile', link: '/architecture/resolution' },
          { text: 'レジストリ・キャッシュ・配置', link: '/architecture/storage' },
          { text: 'CLI・ランナー・スクリプト', link: '/architecture/runtime' },
          { text: 'モジュール対応表', link: '/architecture' },
          { text: '開発・テスト', link: '/development' },
        ] },
      ],
      '/': [
        { text: 'はじめに', items: [
          { text: 'トップ', link: '/' },
          { text: 'インストール', link: '/installation' },
          { text: 'ドキュメント一覧', link: '/README' },
        ] },
        { text: '使い方', items: [
          { text: 'コマンド', link: '/reference/cli/' },
          { text: '設定ファイル', link: '/reference/npmrc' },
          { text: 'workspaces と依存ソース', link: '/dependency-sources' },
          { text: 'JSR と URL import', link: '/jsr-and-url-imports' },
          { text: '認証と公開', link: '/publishing' },
          { text: 'ランナー', link: '/runner' },
        ] },
        { text: '深掘り', collapsed: true, items: [
          { text: '互換性', link: '/compatibility' },
          { text: 'セキュリティと診断', link: '/security-and-diagnostics' },
          { text: 'アーキテクチャ', link: '/architecture/' },
        ] },
      ],
    },
    outline: { level: [2, 3], label: 'このページの目次' },
    docFooter: { prev: '前へ', next: '次へ' },
    lastUpdated: { text: '最終更新' },
    editLink: { pattern: 'https://github.com/taisan11/tnpm/edit/main/docs/:path', text: 'GitHub でこのページを編集' },
    socialLinks: [{ icon: 'github', link: 'https://github.com/taisan11/tnpm' }],
    footer: { message: 'tnpm は npm との完全互換を目指すものではありません。', copyright: 'Documentation for tnpm' },
  },
})
