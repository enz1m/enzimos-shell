import json
import sys


class Notifications:
    def __init__(self, config, grants, send):
        self.send = send
        self.allowed = 'notifications.read' in grants
        self.collecting = bool(config['collect']) and self.allowed
        self.items = []
        self.total = 0
        self.request_id = 0
        self.revision = 0
        self.status = 'Waiting for the notification service.'
        self.subscribe()
        self.snapshot()

    def subscribe(self):
        self.items = []
        self.total = 0
        if not self.allowed:
            self.status = 'Notification read permission was not granted.'
            return
        self.request_id += 1
        self.send({'type': 'request', 'id': self.request_id,
                   'request': {'operation': 'notifications.subscribe',
                               'enabled': self.collecting}})
        self.status = ('Waiting for current previews.' if self.collecting
                       else 'Subscription paused; previews cleared.')

    def snapshot(self):
        self.revision += 1
        nodes = [
            {'type': 'text', 'key': 'status', 'text': self.status[:512], 'tone': 'muted'},
            {'type': 'toggle', 'key': 'collect', 'id': 'collect',
             'label': 'Receive previews', 'value': self.collecting},
        ]
        for item in self.items:
            key = item['id']
            nodes.append({'type': 'column', 'key': 'entry:' + key, 'children': [
                {'type': 'text', 'key': 'title:' + key,
                 'text': (item['source'] + ' — ' + item['title'])[:512], 'tone': 'title'},
                {'type': 'text', 'key': 'preview:' + key,
                 'text': item['preview'][:256], 'tone': 'normal'},
            ]})
        self.send({'type': 'snapshot', 'revision': self.revision, 'elements': [{
            'id': 'feed', 'icon': 'bell', 'label': f'{self.total} notices',
            'tooltip': 'Read-only current notification previews',
            'panel': {'title': 'Notification previews', 'width': 400,
                      'height': 600, 'nodes': nodes},
        }]})

    def handle(self, message):
        kind = message['type']
        if kind == 'ping':
            self.send({'type': 'pong', 'sequence': message['sequence']})
            return
        if kind == 'config':
            self.collecting = bool(message['config']['collect']) and self.allowed
            self.subscribe()
        elif kind == 'action':
            if message['action'] != 'collect':
                return
            self.collecting = bool(message['value']) and self.allowed
            self.subscribe()
        elif kind == 'provider':
            data = message['data']
            if data.get('provider') != 'notifications':
                return
            if data['event'] == 'paused':
                self.items = []
                self.total = 0
                self.status = 'Subscription paused; previews cleared.'
            elif data['event'] == 'snapshot' and self.collecting:
                self.items = data['items'][:8]
                self.total = data['total']
                self.status = (f'{len(self.items)} of {self.total} current notifications. '
                               'Previews only; use the center for actions.'
                               if data['available'] else 'Notification service is unavailable.')
            else:
                return
        elif kind == 'response':
            if message['result']['ok']:
                return
            self.status = message['result']['error'] or 'Subscription was refused.'
        else:
            return
        self.snapshot()


def main():
    def send(value):
        print(json.dumps(value, separators=(',', ':')), flush=True)
    send({'type': 'hello', 'protocol': 1, 'plugin': 'notification-feed'})
    initial = json.loads(sys.stdin.readline())
    notifications = Notifications(initial['config'], initial.get('grants', []), send)
    for line in sys.stdin:
        notifications.handle(json.loads(line))


if __name__ == '__main__':
    main()
