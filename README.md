# Network Audio Bridge

UDP経由でオーディオをリアルタイム送受信するターミナルアプリ。起動するとウィザードが開き、矢印キーで選んでいくだけで繋がります。

## バイナリ一覧

| バイナリ | 説明 |
|----------|------|
| `nab` / `nab.exe` | 送信・受信・双方向の全モード対応（オリジナル） |
| `nab-recv` / `nab-recv.exe` | **受信専用 v2** — アダプティブSRCによるクロックドリフト補正。音切れしにくい |
| `nab-live` | **LiveKit送信用** — REAPER Master/NAB Tap またはCoreAudio入力を48kHz Opus/WebRTCでVPSへ送る |

---

## インストール（1行コマンド）

> **設計方針**: Windows = 送信機専用（Pyramix 等）、Mac = 受信機。双方向が必要な場合のみ Mac 側で `nab` を「双方向」モードで起動。
> **配置場所**: インストール先は **デスクトップの「NAB」フォルダ**。アンインストールはフォルダごと削除するだけ。

### Windows — PowerShell に貼り付けて実行

```powershell
$d="$HOME\Desktop\NAB"; New-Item -Force -ItemType Directory $d|Out-Null; @("nab.exe","start.bat") | ForEach-Object { curl.exe -sSL "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/$_" -o "$d\$_" }; $ws=New-Object -ComObject WScript.Shell; $s=$ws.CreateShortcut("$HOME\Desktop\Start NAB.lnk"); $s.TargetPath="$d\start.bat"; $s.WorkingDirectory=$d; $s.Save(); Write-Host "Done. Desktop\NAB folder and Start NAB shortcut created."
```

作られるもの:
- `Desktop\NAB\` フォルダ（`nab.exe` + `start.bat`）
- `Desktop\Start NAB.lnk`（1クリック起動ショートカット）

> PowerShell 5.1 はコードページが CP932（日本語 Shift_JIS）なので、コマンド内で日本語を使うと文字化け・ファイル名エラーになります。ショートカット名と完了メッセージは ASCII のみにしてあります（nab.exe 内の日本語UIは問題なく動きます）。

更新するときは同じ1行を再実行（`nab.exe` は終了させてから）。

---

### Mac — ターミナルに貼り付けて実行

```bash
d=~/Desktop/NAB && mkdir -p "$d" && \
for f in nab nab-recv nab.command nab-recv.command; do \
  curl -fsSL "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/$f" -o "$d/$f"; \
done && \
chmod +x "$d/nab" "$d/nab-recv" "$d/nab.command" "$d/nab-recv.command" && \
xattr -d com.apple.quarantine "$d"/* 2>/dev/null; \
echo "完了！デスクトップの「NAB」フォルダにインストールしました"
```

作られるもの（すべて `~/Desktop/NAB/` 内）:
- `nab`（送受信・双方向バイナリ）+ `nab.command`（ランチャー）
- `nab-recv`（受信専用・高品質バイナリ）+ `nab-recv.command`（ランチャー）

フォルダを開いて `.command` をダブルクリックで起動。（初回は右クリック→開く）

更新するときは同じ1行を再実行（`nab` / `nab-recv` は終了させてから）。

---

## nab — 送受信アプリの使い方

起動するとウィザードが開きます。

```
① モード選択（↑↓ + Enter）
   ・送信   — このデバイスの音を相手に送る
   ・受信   — 相手の音をこのデバイスで聴く
   ・双方向 — 送受信を同時に行う

② サンプルレート選択（↑↓ + Enter）
   44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz
   ※ 送信側と受信側で必ず同じレートを選ぶこと

③ デバイス選択（↑↓ + Enter）

④ 相手のIPアドレス入力（送信・双方向のみ）
   例: 192.168.1.100

⑤ 自動で接続開始 → 状態が画面に表示されます
```

### 操作キー（実行中）

| キー | 動作 |
|------|------|
| `+` / `-` | ジッターバッファ ±50ms |
| `q` × 2 | 終了（1秒以内に2回） |

---

## nab-recv — 受信機 v2 の使い方

音切れが発生する場合にこちらを使います。送信側は従来の `nab` のまま変更不要。

```
① サンプルレート選択（↑↓ + Enter）
   44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz
   ※ 送信機側と同じレートを選ぶこと

② 出力デバイス選択（↑↓ + Enter）

③ ポート番号入力
   空白のまま Enter → デフォルト 8000 を使用

④ 自動で接続開始 → 状態が画面に表示されます
```

### 操作キー（実行中）

| キー | 動作 |
|------|------|
| `+` / `-` | ジッターターゲット ±50ms（50〜2000ms） |
| `]` | **P-Gain × 2**（ドリフト補正を強くする） |
| `[` | **P-Gain ÷ 2**（ドリフト補正を弱くする） |
| `q` × 2 | 終了（1秒以内に2回） |

### P-Gain（比例ゲイン）について

P-Gain はクロックドリフトの補正速度を調整するパラメータです。

```
表示例:
Drift: +42 ppm  │  ratio: 1.0000420  │  Jitter: 300 ms (+/-)  │  P-Gain: 3.00e-7 ([/])
```

| P-Gain | 特性 |
|--------|------|
| `1e-8` （最小） | 超スロー。変化が穏やか。収束に数分かかる |
| `3e-7` （デフォルト） | 標準。100ppm のドリフトに対して約10〜30秒で収束 |
| `1e-5` （最大） | アグレッシブ。早く収束するが、大きなバッファ変動があると不安定になりやすい |

**調整の目安：**
- 起動直後に音切れが続く → `]` キーでゲインを上げる
- 音が揺れる・ピッチが不安定に感じる → `[` キーでゲインを下げる
- 安定したら、その設定のままにしておく

---

## nab-live + NAB Tap — REAPER MasterをLiveKitへ送る

REAPERのMaster FXまたはMonitor FXに `NAB Tap` VST3プラグインを挿し、別プロセスの `nab-live` がその音を受けてLiveKitへ送ります。

設計:

```
REAPER Master -> NAB Tap plugin -> ~/Library/Caches/KenichiNAB/nab-tap.sock -> nab-live -> LiveKit/VPS
```

重要:

- REAPERでは `VST3: NAB Tap (Kenichi Kawabata)` を使います。
- `NAB Tap` はpass-throughです。音は加工しません。
- プラグイン内ではOpus変換やLiveKit接続をしません。
- Opus変換、SRC、LiveKit送信、再接続、表示は `nab-live` 側で行います。
- 96kHzのREAPERセッションも想定し、`nab-live` 内で48kHz stereoへ変換します。
- LiveKit Rust SDKで公開されている音声保護設定として、bitrate / RED / DTX / queueをCLIから設定できます。Opus FECそのものは現SDKの公開オプションではありません。
- 通常はデスクトップの `NAB Live Sender.command` を使います。設定を選びたい時だけ `NAB Live Wizard.command` を使います。

起動:

```bash
nab-live --source plugin
```

引数なしで起動すると、旧NAB風の軽量ウィザードが開きます。上下キーとEnterで `REAPER Master Plugin - NAB Tap` を選べます。

主な送信設定:

```bash
nab-live --source plugin --profile stable-music
nab-live --source plugin --profile hi-fi-music
nab-live --source plugin --profile low-bandwidth --bitrate 96000
nab-live --source test-tone --test-tone-hz 1000 --test-tone-dbfs=-18
nab-live --source plugin --disable-red
nab-live --source plugin --enable-dtx
```

プロファイル:

| profile | bitrate | RED | DTX | LiveKit queue | 用途 |
|---------|---------|-----|-----|---------------|------|
| stable-music | 160 kbps | on | off | 1200 ms | 標準。まず切れにくさ優先 |
| hi-fi-music | 256 kbps | on | off | 1000 ms | 安定回線で音質優先 |
| speech | 64 kbps | on | on | 800 ms | 会話・確認用 |
| low-bandwidth | 96 kbps | on | off | 1500 ms | 厳しい回線で音楽を切らさない方向 |
| max-quality-lab | 510 kbps | off | off | 1000 ms | 実験用。通常運用では使わない |

Mac miniでの通常手順:

1. `NAB Tap Installer.command` を一度実行する。
2. REAPERを起動し、Master FXまたはMonitor FXに `VST3: NAB Tap (Kenichi Kawabata)` を挿す。
3. `NAB Live Sender.command` を開いたままにする。
4. `NAB Live Status.command` で RTP packets/bytes が増えることを見る。
5. ブラウザで `https://livekit.kenichi-kawabata.com/` を開き、数字の合言葉で `Listen`。
6. listener接続後、`NAB Live Status.command` で `LIVE AUDIO IS MOVING TO A LISTENER NOW.` を見る。

安全設計:

- `NAB Tap Installer.command` はSenderを起動せず、VST3 / nab-live binary / Sender command / Status command / LiveKit token API を確認できます。`INSTALL` を入力した時だけVST3を再インストールします。
- `NAB Live Sender.command` は既存senderがある場合、二重起動せずPIDと状態だけ表示します。
- `nab-live` 本体にも room/identity lock と NAB Tap socket lock があります。
- `kill` / SIGTERM 時も lock/socket を掃除し、再起動時に古いsocketで詰まらないようにしています。
- `NAB Live Status.command` は接続だけでなく、2秒間の `tap_packets` / `captured_frames` / `sent_frames` / RTP packets / RTP bytes / RMS / peak の増加を見ます。
- `NAB Live Status.command` は `subscriber_count` と `listener_identities` を表示します。listenerがいない場合は「LiveKitには届いているがlistenerなし」と分けて表示します。
- `NAB Live Status.command` は `VST3 connected`、`frames_dropped_total`、`last_error`、ログファイルパスも表示します。
- `https://livekit.kenichi-kawabata.com/` は購読専用ページです。listener token は `canPublish=false`, `canSubscribe=true`, 短寿命TTLです。
- listen pageは受信packet/bytes/jitter/audio level/audio element状態を表示します。

プラグインのビルド:

```bash
cmake -S plugins/nab-tap -B plugins/nab-tap/build -DCMAKE_BUILD_TYPE=Release
cmake --build plugins/nab-tap/build --config Release
```

---

## 仕様

| 項目 | nab | nab-recv | nab-live |
|------|-----|----------|----------|
| モード | 送信 / 受信 / 双方向 | 受信専用 | LiveKit送信 |
| サンプルレート | 44.1〜192 kHz（6択） | 44.1〜192 kHz（6択） | 入力48〜192 kHz想定、出力48 kHz |
| チャンネル | ステレオ（2ch） | ステレオ（2ch） | ステレオ（2ch） |
| プロトコル | UDP | UDP（同一フォーマット） | WebRTC/Opus/LiveKit |
| ポート | 8000（固定） | 起動時に指定（デフォルト8000） | LiveKit設定に依存 |
| ドリフト補正 | なし（ジッターバッファのみ） | **アダプティブSRC（rubato）** | SRC 48kHz固定送信 |

---

## ネットワーク設定

- 同じLAN内で使用してください（直結LANケーブルでも動作します）
- ファイアウォールでUDPポート8000を開放してください
- 送信側・受信側のサンプルレートを必ず揃えてください
