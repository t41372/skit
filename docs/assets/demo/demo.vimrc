" ============================================================
"  demo.vimrc — copied to /root/.vimrc in the recording image.
"
"  Mirrors the maintainer's own ~/.vimrc so the demo's "edit script"
"  scene (skit's `e` opens $EDITOR, here vim) looks like a real setup,
"  not a bare `vi`. Keep this roughly in sync with your local vimrc.
" ============================================================

" ---------- 基本行為 ----------
set nocompatible          " 關掉 vi 相容模式，啟用 vim 完整功能
filetype plugin indent on " 根據檔案類型自動載入縮排/外掛規則
syntax on                 " 開啟語法高亮

" ---------- 顯示 ----------
set number                " 顯示行號
set relativenumber        " 相對行號（配合 number 變成混合模式，跳行超方便；不喜歡可刪）
set cursorline            " 高亮游標所在的那一行
set ruler                 " 右下角顯示游標位置
set showcmd               " 右下角顯示正在輸入的指令
set laststatus=2          " 總是顯示狀態列
set wildmenu              " 指令列（:）自動補全時顯示候選選單
set scrolloff=5           " 游標上下至少保留 5 行，捲動時看得到上下文
set showmatch             " 輸入括號時短暫跳到配對的另一半
set termguicolors         " 啟用 24-bit 真彩色（現代終端機幾乎都支援）

" ---------- 縮排 ----------
set expandtab             " Tab 轉成空格
set tabstop=4             " 一個 Tab 顯示為 4 格
set shiftwidth=4          " 自動縮排 / >> << 每次 4 格
set softtabstop=4         " 編輯時 Tab/Backspace 以 4 格為單位
set autoindent            " 換行時沿用上一行縮排
set smartindent           " 根據語法做更聰明的縮排

" ---------- 搜尋 ----------
set ignorecase            " 搜尋預設忽略大小寫
set smartcase             " 但若搜尋字串含大寫，則區分大小寫
set incsearch             " 邊打邊即時定位
set hlsearch              " 高亮所有符合的結果

" ---------- 編輯體驗 ----------
set mouse=a               " 允許滑鼠（選取/移動游標/捲動）
set clipboard=unnamed     " 與系統剪貼簿共用（yank 出去、貼進來）
set backspace=indent,eol,start " 讓 Backspace 能正常刪除
set hidden                " 允許有未儲存的 buffer 時切換檔案
set history=1000          " 記住更多指令/搜尋歷史
set undofile              " 關檔後仍保留 undo 歷史（存到 undodir）
set encoding=utf-8        " 內部編碼用 UTF-8
set nowrap                " 不自動折行（長行往右捲；想折行改成 set wrap）

" ---------- 檔案安全 ----------
set autoread              " 檔案在外部被改動時自動重新載入
set noswapfile            " 不產生 .swp 暫存檔（單機使用通常用不到）

" 把 undo 檔統一放到一個資料夾，避免到處拉屎
if !isdirectory($HOME . '/.vim/undo')
  call mkdir($HOME . '/.vim/undo', 'p')
endif
set undodir=~/.vim/undo

" ---------- 快捷鍵 ----------
" 用空白鍵當 leader（自訂快捷鍵的前綴）
let mapleader = ' '
" 按 <leader> + 空白 清掉搜尋高亮
nnoremap <leader><space> :nohlsearch<CR>

" ---------- 配色 ----------
" vim 內建的深色主題，穩定好看；想換可上網找 colorscheme
set background=dark
silent! colorscheme habamax
