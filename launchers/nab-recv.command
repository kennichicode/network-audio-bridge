#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$SCRIPT_DIR/nab-recv"

if [ ! -f "$BIN" ]; then
    echo ""
    echo "  nab-recv が同じフォルダに見つかりません。"
    echo "  インストールが不完全の可能性があります。"
    echo "  https://github.com/kennichicode/network-audio-bridge"
    echo ""
    read -p "  Enter で閉じます"
    exit 1
fi

chmod +x "$BIN"
"$BIN"
STATUS=$?
if [ $STATUS -ne 0 ]; then
    echo ""
    echo "===================================================="
    echo "  エラーで終了しました"
    echo "  ログ: \$HOME/.nab/log.txt"
    echo "===================================================="
    read -p "  Enter で閉じます"
fi
