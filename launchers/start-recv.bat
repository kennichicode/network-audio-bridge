@echo off
title Network Audio Bridge - Receiver v2
cd /d "%~dp0"
if exist nab-recv.exe (
    nab-recv.exe
) else (
    echo.
    echo  nab-recv.exe がこのフォルダに見つかりません。
    echo  以下からダウンロードして同じフォルダに置いてください：
    echo  https://github.com/kennichicode/network-audio-bridge/releases
    echo.
    pause
)
