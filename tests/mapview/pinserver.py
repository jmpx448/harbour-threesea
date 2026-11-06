# SPDX-License-Identifier: AGPL-3.0-only
"""The test's own HTTPS server for scripts/test-mapview.sh, standing in for every host the map might
reach (the target's /etc/hosts points them here). It writes one line per event to the log: "tcp"
for each connection, "tls <server name>" after a handshake, "http <host> <path>" for each request.
/styles/test/style.json answers with the test style, /redirect with a redirect to another host.

  python3 -I pinserver.py <cert.pem> <key.pem> <style.json> <log> <port>
"""
import socket
import ssl
import sys
import threading


def main(cert, key, style, log_path, port):
    body = open(style, "rb").read()
    log = open(log_path, "a", buffering=1)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    names = {}

    def remember(sock, name, _context):
        names[id(sock)] = name

    context.sni_callback = remember
    # Both loopback addresses (the names point at both), and nothing else.
    listeners = []
    for family, address in ((socket.AF_INET, "127.0.0.1"), (socket.AF_INET6, "::1")):
        listener = socket.socket(family, socket.SOCK_STREAM)
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind((address, port))
        listener.listen(16)
        listeners.append(listener)

    def serve(conn):
        log.write("tcp\n")
        try:
            tls = context.wrap_socket(conn, server_side=True)
        except (ssl.SSLError, OSError):
            conn.close()
            return
        log.write(f"tls {names.get(id(tls), names.get(id(conn), ''))}\n")
        try:
            request = b""
            while b"\r\n\r\n" not in request and len(request) < 65536:
                chunk = tls.recv(4096)
                if not chunk:
                    break
                request += chunk
            if not request:
                return
            lines = request.decode("latin-1").split("\r\n")
            path = lines[0].split(" ")[1] if len(lines[0].split(" ")) > 1 else ""
            host = next((l.split(":", 1)[1].strip() for l in lines[1:] if l.lower().startswith("host:")), "")
            log.write(f"http {host} {path}\n")
            if path == "/styles/test/style.json":
                head = f"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {len(body)}\r\n"
                tls.sendall(head.encode() + b"Connection: close\r\n\r\n" + body)
            elif path == "/redirect":
                tls.sendall(b"HTTP/1.1 302 Found\r\nLocation: https://other.example/styles/test/style.json\r\n"
                            b"Content-Length: 0\r\nConnection: close\r\n\r\n")
            else:
                tls.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        except (ssl.SSLError, OSError):
            pass
        finally:
            tls.close()

    def accept(listener):
        while True:
            conn, _ = listener.accept()
            threading.Thread(target=serve, args=(conn,), daemon=True).start()

    for listener in listeners[1:]:
        threading.Thread(target=accept, args=(listener,), daemon=True).start()
    accept(listeners[0])


main(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5]))
