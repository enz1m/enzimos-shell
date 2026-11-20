import json
import sys


def send(value):
    print(json.dumps(value, separators=(",", ":")), flush=True)


send({"type": "hello", "protocol": 1, "plugin": "app-dock"})
sequence = 0
revision = 0
granted = False
for line in sys.stdin:
    message = json.loads(line)
    if message["type"] in ("start", "config"):
        revision += 1
        send({"type": "snapshot", "revision": revision, "elements": [{
            "id": "applications", "icon": "app-window", "label": "",
            "tooltip": "App dock", "panel": None,
        }]})
        if message["type"] == "start":
            granted = {"shell.bar", "applications.surface"}.issubset(message["grants"])
        if granted:
            sequence += 1
            send({"type": "request", "id": sequence, "request": {
                "operation": "applications.dock", "element": "applications",
            }})
    elif message["type"] == "ping":
        send({"type": "pong", "sequence": message["sequence"]})
