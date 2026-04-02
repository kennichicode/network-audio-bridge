#!/bin/bash
cd "$(dirname "$0")"
if [ -f ./nab ]; then
    chmod +x ./nab
    ./nab
else
    echo ""
    echo "  nab がこのフォルダに見つかりません。"
    echo "  以下からダウンロードして同じフォルダに置いてください："
    echo "  https://github.com/kennichicode/network-audio-bridge/releases"
    echo ""
    read -p "  Enterキーで閉じます"
fi
