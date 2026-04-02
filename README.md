# Network Audio Bridge

UDP経由でオーディオをリアルタイム送受信するターミナルアプリ。起動するとウィザードが開き、矢印キーで選んでいくだけで繋がります。

---

## インストール（1行コマンド）

### Windows — PowerShell に貼り付けて実行

```powershell
$d="$HOME\NetworkAudioBridge"; New-Item -Force -ItemType Directory $d|Out-Null; irm "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/nab.exe" -OutFile "$d\nab.exe"; irm "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/start.bat" -OutFile "$d\start.bat"; $ws=New-Object -ComObject WScript.Shell; $s=$ws.CreateShortcut("$HOME\Desktop\Network Audio Bridge.lnk"); $s.TargetPath="$d\start.bat"; $s.WorkingDirectory=$d; $s.Save(); Write-Host "完了！デスクトップに起動ショートカットを作りました"
```

実行後、デスクトップに **「Network Audio Bridge」** ショートカットが作られます。それをダブルクリックするだけで起動します。

---

### Mac — ターミナルに貼り付けて実行

```bash
mkdir -p ~/NetworkAudioBridge && curl -L "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/nab" -o ~/NetworkAudioBridge/nab && curl -L "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/start.command" -o ~/NetworkAudioBridge/start.command && chmod +x ~/NetworkAudioBridge/nab ~/NetworkAudioBridge/start.command && cp ~/NetworkAudioBridge/start.command ~/Desktop/ && echo "完了！デスクトップに start.command が置かれました"
```

実行後、デスクトップに **「start.command」** が置かれます。それをダブルクリックするだけで起動します。
（初回のみ「開発元を確認できない」と出たら → 右クリック → 開く）

---

## 使い方

起動すると自動でウィザードが開きます。

```
① モード選択
   ↑↓キーで選んで Enter
   ・送信   — このデバイスの音を相手に送る
   ・受信   — 相手の音をこのデバイスで聴く
   ・双方向 — 送受信を同時に行う

② デバイス選択
   ↑↓キーで選んで Enter
   （入力デバイス・出力デバイスそれぞれ選択）

③ 相手のIPアドレス入力（送信・双方向の場合のみ）
   192.168.1.100 のように入力して Enter

④ 自動で接続開始 → 状態が画面に表示されます
   q キーで終了
```

**必ず送信側と受信側の両方を起動してください。** 片方だけでは音は流れません。

---

## 仕様

- サンプルレート: 48kHz
- チャンネル数: ステレオ（2ch）
- プロトコル: UDP
- ポート: 8000（固定）
