# Network Audio Bridge

UDP経由でオーディオをリアルタイム送受信するCLIツール。

## Windows へのインストール

PowerShell に以下を貼り付けるだけ：

```powershell
irm https://github.com/kennichicode/network-audio-bridge/releases/latest/download/nab.exe -OutFile "$HOME\nab.exe"
```

起動確認：

```powershell
& "$HOME\nab.exe" --list-devices
```

## macOS へのインストール

```bash
curl -L https://github.com/kennichicode/network-audio-bridge/releases/latest/download/nab -o ~/nab && chmod +x ~/nab
```

起動確認：

```bash
~/nab --list-devices
```

## 使い方

```bash
# デバイス一覧を表示
nab --list-devices

# 送信側（Mac A）
nab -m send --send-to 192.168.1.XXX:8000

# 受信側（Mac B / Windows）
nab -m recv --listen-on 0.0.0.0:8000

# 双方向（Duplex）
nab -m duplex

# デバイスを指定する場合
nab -m send --send-to 192.168.1.XXX:8000 --input-device "BlackHole 2ch"
```

TUIが起動します。`q` で終了。
