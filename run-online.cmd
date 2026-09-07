@echo off
setlocal
chcp 65001 >nul
cd /d "%~dp0"
title カタン 対戦サーバ

echo ============================================
echo   カタン  友達と対戦するサーバ
echo ============================================
echo.

rem --- 前回の残りが居たら止める -------------------------------------------
rem   ウィンドウを閉じ損ねた時に、古いサーバがポートを掴んだままだと
rem   「ポート 8788 を開けません」で起動できなくなる。先に掃除しておく
taskkill /f /im catan-server.exe >nul 2>&1

rem --- 必要なものを揃える -------------------------------------------------
if not exist "web\catan_wasm.wasm" (
  echo [1/2] 盤のエンジンを組み立てています...
  call cargo build -p catan-wasm --target wasm32-unknown-unknown --release || goto :err
  copy /y "target\wasm32-unknown-unknown\release\catan_wasm.wasm" "web\" >nul || goto :err
)
if not exist "target\release\catan-server.exe" (
  echo [2/2] サーバを組み立てています。初回は数分かかります...
  call cargo build --release -p catan-server || goto :err
)

rem --- 同じ家の中から入るためのアドレス -----------------------------------
set LAN=
for /f "tokens=2 delims=:" %%A in ('ipconfig ^| findstr /c:"IPv4"') do (
  if not defined LAN set LAN=%%A
)
set LAN=%LAN: =%

echo.
echo  --------------------------------------------------------
echo   自分の PC で遊ぶ            http://localhost:8788/
if defined LAN echo   同じ家の Wi-Fi の友達      http://%LAN%:8788/
echo  --------------------------------------------------------
echo.

rem --- 外の友達にも見せる（あれば自動で立てる）---------------------------
where cloudflared >nul 2>&1
if %errorlevel%==0 (
  echo   外の友達にも見せるための入口を用意しています...
  start "カタン 外向けの入口" cmd /k "echo このウィンドウに出る https://....trycloudflare.com が、外の友達に渡すアドレスです。 && echo 閉じると外からは入れなくなります。 && echo. && cloudflared tunnel --url http://localhost:8788"
  echo   別のウィンドウに出る https://....trycloudflare.com を友達に渡してください。
) else (
  echo   ※ 外の友達にも見せたい場合は cloudflared が要ります。
  echo      https://github.com/cloudflare/cloudflared/releases/latest
  echo      から cloudflared-windows-amd64.exe を落として
  echo      cloudflared.exe という名前でこのフォルダに置き、もう一度実行してください。
)

echo.
echo   ブラウザで上のアドレスを開き、「友達と遊ぶ部屋を作る」を押すと
echo   友達に送るリンクが出ます。
echo.
echo   止めるときは、このウィンドウの ×  を押すだけで大丈夫です。
echo   （外向けの入口を開いている場合は、そちらのウィンドウも閉じてください）
echo.
target\release\catan-server.exe 8788 web
goto :eof

:err
echo.
echo  組み立てに失敗しました。Rust が入っているか確認してください。
pause
