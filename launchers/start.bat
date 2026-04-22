@echo off
chcp 65001 >nul
title Network Audio Bridge
cd /d "%~dp0"

if not exist "%~dp0nab.exe" (
    echo.
    echo   nab.exe が同じフォルダに見つかりません。
    echo   インストールが不完全の可能性があります。
    echo   https://github.com/kennichicode/network-audio-bridge
    echo.
    pause
    exit /b 1
)

"%~dp0nab.exe"
if errorlevel 1 (
    echo.
    echo ====================================================
    echo   エラーで終了しました（上のメッセージを確認）
    echo   ログ: %USERPROFILE%\.nab\log.txt
    echo ====================================================
    pause
)
