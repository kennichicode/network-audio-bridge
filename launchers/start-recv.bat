@echo off
chcp 65001 >nul
title Network Audio Bridge - Receiver v2
cd /d "%~dp0"

set "BIN=%~dp0nab-recv.exe"
set "APP_DIR=%USERPROFILE%\NetworkAudioBridge"

if not exist "%BIN%" (
    if exist "%APP_DIR%\nab-recv.exe" (
        set "BIN=%APP_DIR%\nab-recv.exe"
    ) else (
        echo.
        echo   nab-recv.exe が見つかりません。自動ダウンロードします...
        if not exist "%APP_DIR%" mkdir "%APP_DIR%"
        where curl.exe >nul 2>nul
        if errorlevel 1 (
            echo   curl.exe が見つかりません。Windows 10 1803 以降が必要です。
            echo   手動ダウンロード: https://github.com/kennichicode/network-audio-bridge/releases
            pause
            exit /b 1
        )
        curl.exe -L -o "%APP_DIR%\nab-recv.exe" "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/nab-recv.exe"
        if errorlevel 1 (
            echo   ダウンロードに失敗しました。インターネット接続を確認してください。
            pause
            exit /b 1
        )
        set "BIN=%APP_DIR%\nab-recv.exe"
    )
)

"%BIN%"
