#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_DIR="$HOME/NetworkAudioBridge"

if [ -f "$SCRIPT_DIR/nab" ]; then
    BIN_PATH="$SCRIPT_DIR/nab"
elif [ -f "$APP_DIR/nab" ]; then
    BIN_PATH="$APP_DIR/nab"
else
    echo ""
    echo "  nab が見つかりません。自動ダウンロードします..."
    mkdir -p "$APP_DIR"
    curl -L "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/nab" -o "$APP_DIR/nab"
    if [ $? -ne 0 ]; then
        echo ""
        echo "  ダウンロードに失敗しました。インターネット接続を確認してください。"
        read -p "  Enterキーで閉じます"
        exit 1
    fi
    BIN_PATH="$APP_DIR/nab"
fi

chmod +x "$BIN_PATH"
"$BIN_PATH"
