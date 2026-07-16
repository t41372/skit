#!/usr/bin/env fish
# CJK / emoji in values and comments — byte handling must stay correct (問候 = greeting).

set -q 問候; or set 問候 你好世界        # a CJK variable name and value
set -q EMOJI; or set EMOJI "🚀 deploy"   # an emoji value
set -q CITY; or set CITY 臺北            # 臺北 = Taipei

# self-location hint with a CJK comment: 取得腳本所在目錄
set script_dir (status dirname)

echo "$問候 $EMOJI $CITY $script_dir"
