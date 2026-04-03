#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_DIR="$HOME/NetworkAudioBridge"

if [ -f "$SCRIPT_DIR/nab-recv" ]; then
    BIN_PATH="$SCRIPT_DIR/nab-recv"
elif [ -f "$APP_DIR/nab-recv" ]; then
    BIN_PATH="$APP_DIR/nab-recv"
else
    echo ""
    echo "  nab-recv が見つかりません。自動ダウンロードします..."
    mkdir -p "$APP_DIR"
    curl -L "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/nab-recv" -o "$APP_DIR/nab-recv"
    if [ $? -ne 0 ]; then
        echo ""
        echo "  ダウンロードに失敗しました。インターネット接続を確認してください。"
        read -p "  Enterキーで閉じます"
        exit 1
    fi
    BIN_PATH="$APP_DIR/nab-recv"
fi

chmod +x "$BIN_PATH"
"$BIN_PATH"
