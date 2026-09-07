@echo off
rem 友達と遊ぶときのサーバ。web/ の配信と対戦の口を同じ場所から出す。
rem 使い方: このファイルをダブルクリック。止めるときはウィンドウを閉じる。
cd /d "%~dp0"
if not exist target\release\catan-server.exe (
  echo サーバを組み立てています。初回は数分かかります...
  cargo build --release -p catan-server || goto :err
)
if not exist web\catan_wasm.wasm (
  echo 盤のエンジンがありません。build-web.sh を先に実行してください。
  goto :err
)
echo.
echo   同じ家の中から:  http://localhost:8788/
echo   外の友達にも見せる場合は、別のウィンドウで
echo     cloudflared tunnel --url http://localhost:8788
echo   と実行して、出てきた https:// のアドレスを渡してください。
echo.
target\release\catan-server.exe 8788 web
goto :eof
:err
pause
