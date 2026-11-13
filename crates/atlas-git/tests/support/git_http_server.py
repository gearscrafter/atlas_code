"""HTTP git server for the atlas-git integration tests.

Serves the repositories in a folder with the real `git http-backend` and, if given
`user:token`, requires Basic authentication (answers 401 if it does not match).

Usage: python git_http_server.py <root_folder> [user:token]
Prints the port on the first line of standard output (PORT variable to fix it).
"""
import base64
import gzip
import os
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = sys.argv[1]
EXPECTED = sys.argv[2] if len(sys.argv) > 2 else ""


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, fmt, *args):  # silent: the tests do not read the log
        pass

    def do_GET(self):
        self.handle_git()

    def do_POST(self):
        self.handle_git()

    def authorized(self):
        if not EXPECTED:
            return True
        want = "Basic " + base64.b64encode(EXPECTED.encode()).decode()
        return self.headers.get("Authorization", "") == want

    def handle_git(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        if not self.authorized():
            self.send_response(401)
            self.send_header("WWW-Authenticate", 'Basic realm="git"')
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if self.headers.get("Content-Encoding") == "gzip":
            body = gzip.decompress(body)

        path, _, query = self.path.partition("?")
        env = dict(
            os.environ,
            GIT_PROJECT_ROOT=ROOT,
            GIT_HTTP_EXPORT_ALL="1",
            REQUEST_METHOD=self.command,
            PATH_INFO=path,
            QUERY_STRING=query,
            CONTENT_TYPE=self.headers.get("Content-Type", ""),
            CONTENT_LENGTH=str(len(body)),
            REMOTE_USER="git",
            REMOTE_ADDR="127.0.0.1",
            GIT_COMMITTER_NAME="Servidor",
            GIT_COMMITTER_EMAIL="servidor@localhost",
        )
        result = subprocess.run(["git", "http-backend"], input=body, env=env, capture_output=True)
        out = result.stdout
        split = out.find(b"\r\n\r\n")
        if split < 0:
            split = out.find(b"\n\n")
            head, payload = out[:split], out[split + 2 :]
        else:
            head, payload = out[:split], out[split + 4 :]

        status = 200
        headers = []
        for line in head.decode("latin-1").splitlines():
            key, _, value = line.partition(":")
            if key.lower() == "status":
                status = int(value.strip().split()[0])
            elif key:
                headers.append((key, value.strip()))
        self.send_response(status)
        for key, value in headers:
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


if __name__ == "__main__":
    server = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("PORT", "0"))), Handler)
    print(server.server_address[1], flush=True)
    server.serve_forever()
