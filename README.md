# ことのは — kotonoha

> 小学生〜中学生のための、VTuber 先生と話す英会話練習アプリ。

- **3D 先生**: `@pixiv/three-vrm` で VRM 1.0 アバターを表示、口パク + 表情切替。
- **CLI バックエンド**: `claude` / `gemini` / `codex` のローカル CLI を呼び分け。直接 API は叩かないので、各ツールの既存認証がそのまま使える。
- **音声 I/O**: ブラウザの Web Speech API (STT) +
  - ブラウザ標準 (`speechSynthesis`)、または
  - **Kokoro 82M (ローカル ONNX)** — Apache 2.0 / `af_heart` 系のかわいい英語ボイス、`jf_*` で日本語も。pure-Rust phonemizer (espeak 不要)
- **学年別プロンプト**: `configs/lessons/*.toml` を teravars で render。小学校低学年 / 高学年 / 中学生プリセット同梱。
- **PWA**: スマホ追加で全画面起動。

## 動かす

前提:

- Rust toolchain (`rust-toolchain.toml` 経由で stable が固定)
- `cargo-make` (`cargo install cargo-make`)
- [bun](https://bun.sh)
- 以下のどれか (バックエンドは UI で切替):
  - **API 直叩き (推奨・速い)**: [Gemini API key](https://aistudio.google.com/apikey) を発行して `GEMINI_API_KEY` を env にセット
  - **CLI 経由 (ゼロ設定だが起動 2-5 秒)**:
    - [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude`)
    - [Gemini CLI](https://github.com/google-gemini/gemini-cli) (`gemini`)
    - [Codex CLI](https://github.com/openai/codex) (`codex`)

## cargo install 派 (バイナリだけ)

```pwsh
cargo install kotonoha-server
kotonoha setup-tts                 # Kokoro モデル + ボイス DL
$env:GEMINI_API_KEY = "xxx..."     # API モード使うなら
kotonoha serve
# → http://localhost:7400 — SPA が rust-embed でバイナリに焼かれてるので
#   フロントエンド別起動は不要
```

## 開発する派 (リポジトリを clone)

SPA を編集しながら開発する時は Vite dev server (HMR 込み) を別途立てる:

```pwsh
$env:GEMINI_API_KEY = "xxx..."

# A) backend (port 7400) — embedded SPA は古いままだが API は最新
cargo make server-dev

# B) frontend dev server (port 5173) — HMR、/api/* を 7400 へ proxy
cargo make frontend-dev
```

`http://localhost:5173` を開く (HMR 付き)。SPA を変更したら `cargo make web-build` で `web/dist/` を再ビルドし、`cargo build --release` で再焼き込み。

## Kokoro 音声を使う (オプション)

ブラウザ TTS の声が物足りない時は、ローカルで動く Kokoro 82M に切り替えできます。`configs/kotonoha.toml` の `[voice.kokoro]` で指定したパスにモデルとボイスをダウンロード:

```sh
# cargo install 経由のユーザー (バイナリだけ持ってる人):
kotonoha setup-tts                      # Kokoro 量子化モデル (~92 MB) + 英語ボイス 6
kotonoha setup-tts --all-voices         # 全 54 ボイス
kotonoha setup-tts --full               # Kokoro フル精度モデル (~325 MB)

kotonoha setup-voicevox                 # VOICEVOX core + 日本語キャラ (~200 MB)

# repo を clone した dev 向け (bun 必要、機能は完全等価):
bun run setup:tts
bun run setup:tts -- --all-voices
bun run setup:tts -- --full
```

### 言語別ルーティング

`/api/tts` は文単位で言語を見て自動的に振り分けます:

- **英語**: Kokoro (`misaki-lean` G2P、`jf_alpha` 等の英語ボイス)
- **日本語** (ひらがな / カタカナ / 漢字を含む文): VOICEVOX (`春日部つむぎ` 等)

明示指定したい時はリクエストに `lang: "en" | "ja"` を付けると override 可。

### VOICEVOX ライセンス

VOICEVOX のキャラはそれぞれ個別の利用規約を持ちます。本プロジェクトの初期設定は子供向け OK / 商用 OK のキャラ (春日部つむぎ / 四国めたん / ずんだもん) を pre-load しますが、UI 上で再生中のキャラに対しては **"VOICEVOX:<キャラ名>"** のクレジット表示が必要です。詳しくは [VOICEVOX 公式](https://voicevox.hiroshiba.jp/) を参照。

`configs/kotonoha.toml` の `[voice].tts` を `"kokoro"` に変更して backend 再起動 → UI 右上の TTS セレクタで browser / Kokoro を切替可能。

Kokoro 使用時は **本物の音声解析で口パク**が連動します (Web Audio API analyser)。ブラウザ TTS は sin 波の擬似口パクのみ。

## VRM アバターを置く

`avatars/` ディレクトリに `*.vrm` を入れて画面右上のセレクタから選ぶ。
Booth / VRoid Hub の CC0 / フリーモデルが手軽。
`configs/kotonoha.toml` の `[avatars].default` でデフォルトを指定できる。

## バックエンド切替

画面右上のセレクタで `claude` / `gemini` / `codex` を切り替え。CLI のパスや引数は `configs/kotonoha.toml` の `[backend.*]` で調整可能。

## レッスン (システムプロンプト) を作る

`configs/lessons/<name>.toml` に teravars 形式で書いて、`configs/kotonoha.toml` の `[lesson.<name>]` で参照する:

```toml
# configs/lessons/my-lesson.toml
[vars]
grade        = "高校生 1 年"
level_cefr   = "B1"

system_prompt = """
You are an English teacher for {{ vars.grade }} ({{ vars.level_cefr }}) ...
"""
```

```toml
# configs/kotonoha.toml
[lesson.high-1]
extends = "lessons/my-lesson.toml"
```

## スマホで使う

Vite dev サーバは `host: true` で LAN/Tailscale から到達可。ただし
**音声入力 (Web Speech API) は HTTPS 必須**で、`http://192.168.x.x:5173` のような
平文 URL では silent fail する (= マイクボタン押しても何も起きない)。
キーボード入力だけなら HTTP のままで OK。

### 音声入力を使いたい時の HTTPS 化 (おすすめ順)

**A. Tailscale Funnel (一番簡単)**

```pwsh
tailscale funnel --bg --https=443 5173
```

これで `https://<your-machine>.<tailnet>.ts.net/` がインターネット経由で開ける。
スマホで Tailscale に入る必要はない (Funnel は public)。

**B. ローカルだけで完結したい (Tailscale 使わない)**

```pwsh
# 自己署名証明書を生やす (要 mkcert インストール)
mkcert -install
mkcert localhost 192.168.x.x

# vite.config.ts に証明書を指定して起動
```

スマホ側にも mkcert の root CA をインストールする必要あり (めんどう)。

**C. Cloudflare Tunnel / ngrok**

公開トンネル系。一時的なテスト URL が欲しい時に。

### PWA

ホーム画面に追加すれば全画面起動、起動アイコンも付く。

## ライセンス

MIT.
