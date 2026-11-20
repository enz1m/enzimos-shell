import json
import sys
import time
from collections import OrderedDict


class Clipboard:
    def __init__(self, config, send, clock=time.monotonic):
        self.config = config
        self.send = send
        self.clock = clock
        self.entries = OrderedDict()
        self.collecting = config['collect']
        self.revision = 0
        self.request_id = 0
        self.pending = {}
        self.error = None
        self.request('clipboard.subscribe', enabled=self.collecting)
        self.snapshot()

    def request(self, operation, **fields):
        self.request_id += 1
        self.send({'type': 'request', 'id': self.request_id,
                   'request': {'operation': operation, **fields}})
        return self.request_id

    def release(self, handles):
        for offset in range(0, len(handles), 128):
            self.request('blobs.release', blobs=handles[offset:offset + 128])

    def trim(self):
        now = self.clock()
        removed = [key for key, item in self.entries.items()
                   if now - item['at'] >= self.config['retention_minutes'] * 60]
        for key in removed:
            del self.entries[key]
        limit = self.config['max_mb'] * 1024 * 1024
        used = sum(item['bytes'] for item in self.entries.values())
        while self.entries and (len(self.entries) > 128 or used > limit):
            key, item = self.entries.popitem(last=True)
            removed.append(key)
            used -= item['bytes']
        self.release(removed)
        return bool(removed)

    def snapshot(self):
        self.revision += 1
        def action(key, label, icon, enabled=True):
            return {'type': 'icon_action', 'key': key, 'id': key, 'label': label,
                    'icon': icon, 'enabled': enabled}

        ordered = sorted(self.entries.items(), key=lambda pair: not pair[1]['pinned'])
        nodes = []
        if self.error:
            nodes.append({'type': 'text', 'key': 'error', 'text': self.error[:512], 'tone': 'danger'})
        for key, item in ordered:
            label = ' '.join(item['preview'].split()).encode('utf-8')[:96].decode('utf-8', errors='ignore') or '(Whitespace)'
            nodes.append({'type': 'entry', 'key': key, 'label': label,
                          'thumbnail': key if item['image'] else None,
                          'copy': key, 'delete': 'delete/' + key, 'pin': 'pin/' + key,
                          'pinned': item['pinned']})
        if not self.entries:
            nodes.append({'type': 'text', 'key': 'empty',
                          'text': ('Copy text or a PNG image to begin.' if self.collecting else
                                   'Nothing is collected while paused.'), 'tone': 'muted'})
        footer = []
        if self.error:
            footer.append(action('dismiss-error', 'Dismiss error', 'x'))
        footer.extend([
            # A named switch: an icon alone does not say what it pauses.
            {'type': 'toggle', 'key': 'collect', 'id': 'collect',
             'label': 'Save history', 'value': self.collecting},
            # A labelled button apart from the switch: a bare bin beside
            # "Save history" read as part of it. Pinned entries go too.
            {'type': 'action', 'key': 'clear', 'id': 'clear', 'label': 'Clear history',
             'detail': None, 'icon': None, 'thumbnail': None, 'enabled': bool(self.entries)},
        ])
        height = max(180, 116 + max(1, min(8, len(ordered))) * 40 +
                     (64 if self.error else 0))
        # The bar keeps a stable name: copied text never shows on the bar, where
        # anyone looking at (or sharing) the screen would see it.
        label = 'Clipboard'
        self.send({'type': 'snapshot', 'revision': self.revision, 'elements': [{
            'id': 'history', 'icon': 'clipboard', 'label': label,
            'tooltip': f"Clipboard history · {len(self.entries)} items" +
                       ('' if self.collecting else ' · paused'),
            'panel': {'title': 'Clipboard history', 'width': 400, 'height': height,
                      'nodes': nodes, 'footer': footer, 'entry_list': True},
        }]})

    def handle(self, message):
        changed = self.trim()
        kind = message['type']
        if kind == 'ping':
            self.send({'type': 'pong', 'sequence': message['sequence']})
        elif kind == 'config':
            collect_changed = message['config']['collect'] != self.config['collect']
            self.config = message['config']
            if collect_changed:
                self.collecting = self.config['collect']
            self.request('clipboard.subscribe', enabled=self.collecting)
            self.trim()
            changed = True
        elif kind == 'provider':
            data = message['data']
            if data.get('provider') != 'clipboard':
                return
            event = data['event']
            if event == 'entry' and self.collecting:
                key = data['blob']
                if not data['fresh'] and key not in self.entries:
                    self.release([key])
                    if changed:
                        self.snapshot()
                    return
                old = self.entries.get(key)
                self.entries[key] = {**data, 'at': old['at'] if old else self.clock(),
                                     'pinned': old['pinned'] if old else False}
                self.entries.move_to_end(key, last=False)
                self.trim()
            elif event == 'entry':
                if data['blob'] not in self.entries:
                    self.release([data['blob']])
            elif event == 'removed':
                for key in data['blobs']:
                    self.entries.pop(key, None)
            elif event == 'unavailable':
                self.entries.clear()
                self.pending.clear()
                self.error = data['message']
            elif event == 'error':
                self.error = data['message']
            changed = True
        elif kind == 'action':
            action = message['action']
            if action == 'clear':
                self.release(list(self.entries))
                self.entries.clear()
                self.pending.clear()
            elif action == 'dismiss-error':
                self.error = None
            elif action == 'collect':
                self.collecting = bool(message['value'])
                self.request('clipboard.subscribe', enabled=self.collecting)
            elif action.startswith('delete/') and action[7:] in self.entries:
                key = action[7:]
                del self.entries[key]
                self.release([key])
                self.pending = {request: entry for request, entry in self.pending.items() if entry != key}
            elif action.startswith('pin/') and action[4:] in self.entries:
                self.entries[action[4:]]['pinned'] = bool(message['value'])
            elif action in self.entries:
                if len(self.pending) < 8:
                    request = self.request('clipboard.restore', blob=action,
                                           gesture=message['gesture'])
                    self.pending[request] = action
                else:
                    self.error = 'Clipboard is busy. Try copying again.'
            changed = True
        elif kind == 'response':
            key = self.pending.pop(message['id'], None)
            if key is not None:
                if message['result']['ok'] and key in self.entries:
                    self.entries.move_to_end(key, last=False)
                elif not message['result']['ok']:
                    self.error = message['result']['error'] or 'Entry is no longer available.'
                changed = True
            elif not message['result']['ok']:
                self.error = message['result']['error']
                changed = True
        if changed:
            self.snapshot()


def main():
    def send(value):
        print(json.dumps(value, separators=(',', ':'), ensure_ascii=False), flush=True)
    send({'type': 'hello', 'protocol': 1, 'plugin': 'clipboard'})
    initial = json.loads(sys.stdin.readline())
    clipboard = Clipboard(initial['config'], send)
    for line in sys.stdin:
        clipboard.handle(json.loads(line))


if __name__ == '__main__':
    main()
