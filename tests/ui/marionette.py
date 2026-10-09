"""Minimal Firefox Marionette client: drive a page and take screenshots."""
import base64, json, socket, sys, time

class Browser:
    def __init__(self, port=2828):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=60)
        self.msg_id = 0
        self._read()  # handshake
        self.call("WebDriver:NewSession", {"capabilities": {}})

    def _read(self):
        length = b""
        while not length.endswith(b":"):
            length += self.sock.recv(1)
        n = int(length[:-1]); data = b""
        while len(data) < n:
            data += self.sock.recv(n - len(data))
        return json.loads(data)

    def call(self, command, params=None):
        self.msg_id += 1
        payload = json.dumps([0, self.msg_id, command, params or {}]).encode()
        self.sock.sendall(str(len(payload)).encode() + b":" + payload)
        reply = self._read()
        if reply[2]:
            raise RuntimeError(f"{command}: {reply[2]}")
        return reply[3]

    def go(self, url):
        self.call("WebDriver:Navigate", {"url": url})

    def js(self, script, *args):
        return self.call("WebDriver:ExecuteScript", {"script": script, "args": list(args)}).get("value")

    def wait(self, script, timeout=30):
        end = time.time() + timeout
        while time.time() < end:
            if self.js(script):
                return
            time.sleep(0.2)
        raise TimeoutError(script)

    def screenshot(self, path):
        data = self.call("WebDriver:TakeScreenshot", {"full": True})["value"]
        open(path, "wb").write(base64.b64decode(data))

if __name__ == "__main__":
    b = Browser()
    b.go("data:text/html,<h1 id=t>hello</h1>")
    print("title text:", b.js("return document.getElementById('t').textContent"))
