# 友達と対戦するためのサーバを立てる。
#
# 🔴 バッチ(.cmd)ではなく PowerShell で書いてある。
#    cmd.exe は日本語を含むファイルを括弧の中などで正しく読めず、
#    「'…' は認識されていません」と言って壊れる（実際に踏んだ）。
#    ここは案内文が主役なので、文字を正しく扱える方で書く。

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $root
$Host.UI.RawUI.WindowTitle = "カタン 対戦サーバ"
# サーバ本体は UTF-8 で書き出す。既定のままだと日本語が化けるので合わせる
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

function Say($t, $color = "White") { Write-Host $t -ForegroundColor $color }

Say ""
Say "============================================" Cyan
Say "  カタン  友達と対戦するサーバ" Cyan
Say "============================================" Cyan
Say ""

# --- 前回の残りを片付ける ------------------------------------------------
# 閉じ損ねた古いサーバがポートを掴んだままだと、次が起動できない
Get-Process catan-server -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

# --- 必要な物を組み立てる ------------------------------------------------
if (-not (Test-Path "web\catan_wasm.wasm")) {
    Say "盤のエンジンを組み立てています..." Yellow
    cargo build -p catan-wasm --target wasm32-unknown-unknown --release
    if ($LASTEXITCODE -ne 0) { Say "組み立てに失敗しました。Rust が入っているか確認してください。" Red; pause; exit 1 }
    Copy-Item "target\wasm32-unknown-unknown\release\catan_wasm.wasm" "web\" -Force
}
if (-not (Test-Path "target\release\catan-server.exe")) {
    Say "サーバを組み立てています。初回は数分かかります..." Yellow
    cargo build --release -p catan-server
    if ($LASTEXITCODE -ne 0) { Say "組み立てに失敗しました。Rust が入っているか確認してください。" Red; pause; exit 1 }
}

# --- 同じ家の中から入るためのアドレス -------------------------------------
# 既定ゲートウェイを持つ口だけが「外と繋がっている口」。
# これで選ばないと WSL や Hyper-V の仮想アドレス(172.x)を案内してしまう
$lan = $null
try {
    $lan = (Get-NetIPConfiguration -ErrorAction SilentlyContinue |
        Where-Object { $_.IPv4DefaultGateway } |
        Select-Object -First 1).IPv4Address.IPAddress
} catch {}

Say " --------------------------------------------------------"
Say "  自分の PC で遊ぶ         http://localhost:8788/colonist/" Green
if ($lan) { Say "  同じ家の Wi-Fi の友達    http://${lan}:8788/colonist/" Green }
Say ""
Say "  （前の見た目で遊びたいときは、末尾の colonist/ を外す）" DarkGray
Say " --------------------------------------------------------"
Say ""

# --- 外の友達に渡すアドレス ----------------------------------------------
# cloudflared の出力はログの山で、肝心のアドレスが枠の中に埋もれる。
# ここで拾い出して、大きく出す
$tunnelUrl = $null
$cf = (Get-Command cloudflared -ErrorAction SilentlyContinue).Source
if (-not $cf) {
    # 入れた直後の窓は PATH がまだ古い。登録簿から今の PATH を読み直して探す
    try {
        $dirs = @()
        foreach ($scope in @("HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment", "HKCU:\Environment")) {
            $v = (Get-ItemProperty -Path $scope -Name Path -ErrorAction SilentlyContinue).Path
            if ($v) { $dirs += $v.Split(";") }
        }
        $dirs += @("$env:LOCALAPPDATA\Microsoft\WinGet\Links", "$env:ProgramFiles\Cloudflare\Cloudflared", "$env:ProgramFiles\cloudflared", $root)
        foreach ($d in $dirs) {
            if (-not $d) { continue }
            $try = Join-Path ([Environment]::ExpandEnvironmentVariables($d)) "cloudflared.exe"
            if (Test-Path $try) { $cf = $try; break }
        }
    } catch {}
}
if ($cf) {
    $log = Join-Path $env:TEMP "catan-tunnel.log"
    Remove-Item $log -ErrorAction SilentlyContinue
    Say "外の友達に渡すアドレスを用意しています（15秒ほど）..." Yellow
    Start-Process -FilePath $cf `
        -ArgumentList "tunnel", "--url", "http://localhost:8788" `
        -RedirectStandardError $log -RedirectStandardOutput "$log.out" -WindowStyle Hidden
    foreach ($i in 1..40) {
        Start-Sleep -Milliseconds 700
        if (Test-Path $log) {
            $m = Select-String -Path $log -Pattern "https://[a-z0-9-]+\.trycloudflare\.com" -ErrorAction SilentlyContinue |
                 Select-Object -First 1
            if ($m) { $tunnelUrl = $m.Matches[0].Value; break }
        }
    }
}

if ($tunnelUrl) {
    Say " ========================================================" Green
    Say "   外の友達に渡すアドレス" Green
    Say ""
    Say "     $tunnelUrl/colonist/" Green
    Say ""
    Say "   ↑ まずこれを自分のブラウザで開いてから部屋を作ってください" Green
    Say " ========================================================" Green
    try { Set-Clipboard -Value "$tunnelUrl/colonist/"; Say "   （コピー済みです。そのまま貼り付けられます）" DarkGray } catch {}
} elseif ($cf) {
    Say "  外向けのアドレスを取得できませんでした。" Red
    Say "  詳しくは $env:TEMP\catan-tunnel.log を見てください。" Red
} else {
    Say "  外の友達にも見せたい場合は、一度だけ次を実行してください:" DarkYellow
    Say "    winget install --id Cloudflare.cloudflared" DarkYellow
    Say "  入れたらこのウィンドウを閉じて、もう一度実行してください。" DarkYellow
}

Say ""
Say "  ブラウザで開いて「友達と遊ぶ部屋を作る」を押すと、"
Say "  友達に送るリンクが出ます。"
Say ""
Say "  止めるときは、このウィンドウの × を押すだけで大丈夫です。" DarkGray
Say ""

& "target\release\catan-server.exe" 8788 web
