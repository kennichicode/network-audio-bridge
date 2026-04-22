# Network Audio Bridge

UDP経由でオーディオをリアルタイム送受信するターミナルアプリ。起動するとウィザードが開き、矢印キーで選んでいくだけで繋がります。

## バイナリ一覧

| バイナリ | 説明 |
|----------|------|
| `nab` / `nab.exe` | 送信・受信・双方向の全モード対応（オリジナル） |
| `nab-recv` / `nab-recv.exe` | **受信専用 v2** — アダプティブSRCによるクロックドリフト補正。音切れしにくい |

---

## インストール（1行コマンド）

> **設計方針**: Windows = 送信機専用（Pyramix 等）、Mac = 受信機。双方向が必要な場合のみ Mac 側で `nab` を「双方向」モードで起動。
> **配置場所**: インストール先は **デスクトップの「NAB」フォルダ**。アンインストールはフォルダごと削除するだけ。

### Windows — PowerShell に貼り付けて実行

```powershell
$d="$HOME\Desktop\NAB"; New-Item -Force -ItemType Directory $d|Out-Null; @("nab.exe","start.bat") | ForEach-Object { curl.exe -sSL "https://github.com/kennichicode/network-audio-bridge/releases/latest/download/$_" -o "$d\$_" }; $ws=New-Object -ComObject WScript.Shell; $s=$ws.CreateShortcut("$HOME\Desktop\NAB 送受信.lnk"); $s.TargetPath="$d\start.bat"; $s.WorkingDirectory=$d; $s.Save(); Write-Host "完了！デスクトップに「NAB」フォルダと「NAB 送受信」ショートカットを作りました"
```

作られるもの:
- `Desktop\NAB\` フォルダ（`nab.exe` + `start.bat`）
- `Desktop\NAB 送受信.lnk`（1クリック起動ショートカット）

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

## 仕様

| 項目 | nab | nab-recv |
|------|-----|----------|
| モード | 送信 / 受信 / 双方向 | 受信専用 |
| サンプルレート | 44.1〜192 kHz（6択） | 44.1〜192 kHz（6択） |
| チャンネル | ステレオ（2ch） | ステレオ（2ch） |
| プロトコル | UDP | UDP（同一フォーマット） |
| ポート | 8000（固定） | 起動時に指定（デフォルト8000） |
| ドリフト補正 | なし（ジッターバッファのみ） | **アダプティブSRC（rubato）** |

---

## ネットワーク設定

- 同じLAN内で使用してください（直結LANケーブルでも動作します）
- ファイアウォールでUDPポート8000を開放してください
- 送信側・受信側のサンプルレートを必ず揃えてください
