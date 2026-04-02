@echo off
title Network Audio Bridge
cd /d "%~dp0"
if exist nab.exe (
    nab.exe
) else (
    echo.
    echo  nab.exe がこのフォルダに見つかりません。
    echo  以下からダウンロードして同じフォルダに置いてください：
    echo  https://github.com/kennichicode/network-audio-bridge/releases
    echo.
    pause
)
