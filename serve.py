"""web/ をローカルに配る小さなサーバ。

`python -m http.server` だとブラウザがキャッシュを握り続けて、
app.js や style.css を直したのに画面が変わらないことがある（実際に何度も踏んだ）。
毎回 no-store を返して、常に最新を読ませる。
"""

import functools
import http.server
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
    handler = functools.partial(NoCache, directory="web")
    socketserver.TCPServer.allow_reuse_address = True
    with socketserver.TCPServer(("127.0.0.1", port), handler) as httpd:
        print(f"http://localhost:{port}")
        httpd.serve_forever()
