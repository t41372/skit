import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { uiTranslations } from 'fumadocs-ui/i18n';
import { i18n } from './i18n';
import { appName, gitConfig } from './shared';

// Base path under which the static site is served (see next.config.mjs). Needed
// to reference the site icon from raw markup, which does not get the automatic
// basePath prefix that next/link / metadata icons do.
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/skit';

// Translations for Fumadocs' own layout chrome. English ships via the official
// `uiTranslations()` pack; each other locale registers its overrides here.
export const translations = i18n
  .translations()
  .extend(uiTranslations())
  .add({
    'zh-CN': {
      'Ask AI(AI chat button)': '询问 AI',
      'Back to Home(404 not found page)': '返回首页',
      'Choose a language(language switcher)': '选择语言',
      'Choose a language(language switcher)(aria-label)': '选择语言',
      'Close Banner(banner)(aria-label)': '关闭横幅',
      'Close Search(search dialog)(aria-label)': '关闭搜索',
      'Close Sidebar(aria-label)': '关闭侧边栏',
      'Close Sidebar(sidebar)(aria-label)': '关闭侧边栏',
      'Collapse Sidebar(sidebar)(aria-label)': '折叠侧边栏',
      'Copied Text(code block)(aria-label)': '已复制文本',
      'Copy Anchor Link(heading anchor)(aria-label)': '复制锚点链接',
      'Copy Link(accordion)(aria-label)': '复制链接',
      'Copy Markdown(page actions)': '复制 Markdown',
      'Copy Text(code block)(aria-label)': '复制文本',
      'Dark(theme switcher)(aria-label)': '深色',
      'Default(type table)': '默认值',
      'Edit on GitHub(edit page)': '在 GitHub 上编辑',
      'Hide Sidebar(sidebar)': '隐藏侧边栏',
      'Last updated on(page footer)': '最后更新于',
      'Layout Tab(layout tab trigger)': '布局标签',
      'Light(theme switcher)(aria-label)': '浅色',
      'Next Page(pagination)': '下一页',
      'No Headings(table of contents)': '没有标题',
      'No results found(search dialog)': '没有找到结果',
      'On this page(table of contents)': '本页内容',
      'Open Search(search trigger)(aria-label)': '打开搜索',
      'Open Sidebar(aria-label)': '打开侧边栏',
      'Open Sidebar(sidebar)(aria-label)': '打开侧边栏',
      'Open in ChatGPT(page actions)': '在 ChatGPT 中打开',
      'Open in Claude(page actions)': '在 Claude 中打开',
      'Open in Cursor(page actions)': '在 Cursor 中打开',
      'Open in GitHub(page actions)': '在 GitHub 中打开',
      'Open in Scira AI(page actions)': '在 Scira AI 中打开',
      'Open(page actions)': '打开',
      'Page Not Found(404 not found page)': '找不到页面',
      'Parameters(type table)': '参数',
      'Previous Page(pagination)': '上一页',
      'Prop(type table)': '属性',
      'Read {url}, I want to ask questions about it.(page actions)':
        '请阅读 {url}，我想就此提问。',
      'Returns(type table)': '返回值',
      'Search(search dialog)': '搜索',
      'Search(search trigger)': '搜索',
      'Show Sidebar(sidebar)': '显示侧边栏',
      'System(theme switcher)(aria-label)': '跟随系统',
      'Table of Contents(inline table of contents)': '目录',
      'The page you are looking for might have been removed, had its name changed, or is temporarily unavailable.(404 not found page)':
        '你要找的页面可能已被删除、改名，或暂时无法访问。',
      'Toggle Menu(home layout header)(aria-label)': '切换菜单',
      'Toggle Theme(theme switcher)(aria-label)': '切换主题',
      'Type(type table)': '类型',
      'View as Markdown(page actions)': '以 Markdown 查看',
      displayName: '简体中文',
    },
    'zh-TW': {
      'Ask AI(AI chat button)': '詢問 AI',
      'Back to Home(404 not found page)': '返回首頁',
      'Choose a language(language switcher)': '選擇語言',
      'Choose a language(language switcher)(aria-label)': '選擇語言',
      'Close Banner(banner)(aria-label)': '關閉橫幅',
      'Close Search(search dialog)(aria-label)': '關閉搜尋',
      'Close Sidebar(aria-label)': '關閉側邊欄',
      'Close Sidebar(sidebar)(aria-label)': '關閉側邊欄',
      'Collapse Sidebar(sidebar)(aria-label)': '收合側邊欄',
      'Copied Text(code block)(aria-label)': '已複製文字',
      'Copy Anchor Link(heading anchor)(aria-label)': '複製錨點連結',
      'Copy Link(accordion)(aria-label)': '複製連結',
      'Copy Markdown(page actions)': '複製 Markdown',
      'Copy Text(code block)(aria-label)': '複製文字',
      'Dark(theme switcher)(aria-label)': '深色',
      'Default(type table)': '預設值',
      'Edit on GitHub(edit page)': '在 GitHub 上編輯',
      'Hide Sidebar(sidebar)': '隱藏側邊欄',
      'Last updated on(page footer)': '最後更新於',
      'Layout Tab(layout tab trigger)': '版面標籤',
      'Light(theme switcher)(aria-label)': '淺色',
      'Next Page(pagination)': '下一頁',
      'No Headings(table of contents)': '沒有標題',
      'No results found(search dialog)': '沒有找到結果',
      'On this page(table of contents)': '本頁內容',
      'Open Search(search trigger)(aria-label)': '開啟搜尋',
      'Open Sidebar(aria-label)': '開啟側邊欄',
      'Open Sidebar(sidebar)(aria-label)': '開啟側邊欄',
      'Open in ChatGPT(page actions)': '在 ChatGPT 中開啟',
      'Open in Claude(page actions)': '在 Claude 中開啟',
      'Open in Cursor(page actions)': '在 Cursor 中開啟',
      'Open in GitHub(page actions)': '在 GitHub 中開啟',
      'Open in Scira AI(page actions)': '在 Scira AI 中開啟',
      'Open(page actions)': '開啟',
      'Page Not Found(404 not found page)': '找不到頁面',
      'Parameters(type table)': '參數',
      'Previous Page(pagination)': '上一頁',
      'Prop(type table)': '屬性',
      'Read {url}, I want to ask questions about it.(page actions)':
        '請閱讀 {url}，我想就此提問。',
      'Returns(type table)': '回傳值',
      'Search(search dialog)': '搜尋',
      'Search(search trigger)': '搜尋',
      'Show Sidebar(sidebar)': '顯示側邊欄',
      'System(theme switcher)(aria-label)': '跟隨系統',
      'Table of Contents(inline table of contents)': '目錄',
      'The page you are looking for might have been removed, had its name changed, or is temporarily unavailable.(404 not found page)':
        '你要找的頁面可能已被刪除、改名，或暫時無法存取。',
      'Toggle Menu(home layout header)(aria-label)': '切換選單',
      'Toggle Theme(theme switcher)(aria-label)': '切換主題',
      'Type(type table)': '類型',
      'View as Markdown(page actions)': '以 Markdown 檢視',
      displayName: '繁體中文',
    },
  });

export function baseOptions(locale: string): BaseLayoutProps {
  return {
    nav: {
      // The docs Overview is the front door — there is no separate landing page,
      // so the logo links straight there.
      url: `/${locale}/docs`,
      title: (
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: '0.4rem' }}>
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={`${basePath}/icon.png`} alt="" width={22} height={22} />
          {appName}
        </span>
      ),
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
