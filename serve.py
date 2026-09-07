"""web/ をローカルに配る小さなサーバ。

`python -m http.server` だとブラウザがキャッシュを握り続けて、
app.js や style.css を直したのに画面が変わらないことがある（実際に何度も踏んだ）。
毎回 no-store を返して、常に最新を読ませる。

🔴 **1 本ずつしか捌けないサーバにするな**。`TCPServer` のままだと、
   1 つの接続が詰まっただけで以降の要求が全部止まる（実際に踏んだ。
   ぶら下がった curl 1 本でサーバ全体が無反応になった）。
"""

import functools
import http.server
import os
import socketserver
import sys


class NoCache(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store, must-revalidate")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        super().end_headers()

    def log_message(self, fmt, *args):
        pass  # 静かに


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8787
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "web")
    handler = functools.partial(NoCache, directory=root)
    socketserver.ThreadingTCPServer.allow_reuse_address = True
    socketserver.ThreadingTCPServer.daemon_threads = True
    with socketserver.ThreadingTCPServer(("127.0.0.1", port), handler) as httpd:
        print(f"http://localhost:{port}")
        httpd.serve_forever()
